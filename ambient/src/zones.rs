//! What the video hardware reports: up to 16 picture areas, each with a 256-bin histogram per channel, and how that
//! becomes one colour per area.
//!
//! The layout of [`RawZone`] is LG's `struct v4l2_ext_histogram_data` (0x604 bytes, little-endian on the set). The
//! probe's `-d` dump (`VTH1` records) can be replayed through [`parse_dump`], so the mapping code can be tuned against
//! frames recorded on the set.

use crate::presets::{PRESET0, PRESET1, PRESET2, PRESET3};
use crate::rgb::Rgb;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const BINS: usize = 256;
pub const MAX_ZONES: usize = 16;
pub const RAW_ZONE_BYTES: usize = 4 + 3 * BINS * 2;

/// A rectangle in picture coordinates: (0, 0) top left, (1, 1) bottom right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    pub fn area(&self) -> f32 {
        (self.x1 - self.x0) * (self.y1 - self.y0)
    }

    /// Distance from a point to the nearest point of the rectangle; 0 inside.
    pub fn distance(&self, x: f32, y: f32) -> f32 {
        let dx = (self.x0 - x).max(x - self.x1).max(0.0);
        let dy = (self.y0 - y).max(y - self.y1).max(0.0);
        dx.hypot(dy)
    }
}

/// The hardware's four area sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    /// Plain video, 16 areas.
    #[default]
    Plain = 0,
    /// Top and bottom black bars skipped (letterboxed film), 16 areas.
    TopBottom = 1,
    /// Left and right black bars skipped, 16 areas.
    SideBars = 2,
    /// Games, 14 areas weighted to the edges.
    Games = 3,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Plain, Preset::TopBottom, Preset::SideBars, Preset::Games];

    pub fn from_index(i: u8) -> Option<Preset> {
        Preset::ALL.get(i as usize).copied()
    }

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn rects(self) -> &'static [Rect] {
        match self {
            Preset::Plain => &PRESET0,
            Preset::TopBottom => &PRESET1,
            Preset::SideBars => &PRESET2,
            Preset::Games => &PRESET3,
        }
    }

    pub fn zone_count(self) -> usize {
        self.rects().len()
    }
}

/// Area ids the bar detector reads; the same in presets 0 to 2.
pub const TOP_STRIP: usize = 14;
pub const LEFT_STRIP: usize = 15;
pub const BOTTOM_STRIP: usize = 1;
pub const CENTER: usize = 2;

/// One area as the driver returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawZone {
    pub area_id: u32,
    pub r: [u16; BINS],
    pub g: [u16; BINS],
    pub b: [u16; BINS],
}

impl RawZone {
    pub fn empty(area_id: u32) -> RawZone {
        RawZone { area_id, r: [0; BINS], g: [0; BINS], b: [0; BINS] }
    }

    /// Parses one `struct v4l2_ext_histogram_data` (little-endian, 0x604 bytes).
    pub fn from_bytes(b: &[u8]) -> Option<RawZone> {
        if b.len() < RAW_ZONE_BYTES {
            return None;
        }
        let mut z = RawZone::empty(u32::from_le_bytes(b[0..4].try_into().ok()?));
        let rd = |off: usize, out: &mut [u16; BINS]| {
            for (i, v) in out.iter_mut().enumerate() {
                let p = off + i * 2;
                *v = u16::from_le_bytes([b[p], b[p + 1]]);
            }
        };
        rd(4, &mut z.r);
        rd(4 + BINS * 2, &mut z.g);
        rd(4 + BINS * 4, &mut z.b);
        Some(z)
    }
}

/// How a histogram becomes a colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameParams {
    /// Average the code values (what HyperHDR and Hue Sync do) or the light they stand for.
    pub mean: MeanKind,
    /// Highest code value counted as black when measuring how much of an area is black.
    pub black_code: u8,
}

impl Default for FrameParams {
    fn default() -> Self {
        // Limited-range video black is code 16; a few codes of margin for noise and dithering.
        FrameParams { mean: MeanKind::Code, black_code: 20 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeanKind {
    /// Mean of the 8-bit code values: bright and dark pixels count by their code.
    #[default]
    Code,
    /// Mean in linear light (sRGB-decoded), re-encoded: highlights dominate, as they do to the eye in a dark room.
    Linear,
}

/// One capture: a colour per area plus what the bar detector needs.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneFrame {
    pub preset: Preset,
    pub color: [Rgb; MAX_ZONES],
    /// Fraction of the area's pixels at or under `black_code`, the smallest of the three channels' fractions
    /// (so all three must be mostly black for this to be high). 0 for an empty histogram.
    pub black: [f32; MAX_ZONES],
    /// Pixels the histogram counted (red channel); 0 means the area reported nothing.
    pub count: [u32; MAX_ZONES],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The driver returned a different number of areas than the preset has.
    ZoneCount { expected: usize, got: usize },
    /// An area id is out of range or repeated.
    AreaId(u32),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::ZoneCount { expected, got } => write!(f, "expected {expected} areas, got {got}"),
            FrameError::AreaId(id) => write!(f, "bad or repeated area id {id}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl ZoneFrame {
    pub fn blank(preset: Preset) -> ZoneFrame {
        ZoneFrame { preset, color: [Rgb::BLACK; MAX_ZONES], black: [0.0; MAX_ZONES], count: [0; MAX_ZONES] }
    }

    pub fn from_histograms(preset: Preset, zones: &[RawZone], p: FrameParams) -> Result<ZoneFrame, FrameError> {
        let n = preset.zone_count();
        if zones.len() != n {
            return Err(FrameError::ZoneCount { expected: n, got: zones.len() });
        }
        let mut f = ZoneFrame::blank(preset);
        let mut seen = [false; MAX_ZONES];
        let lut = linear_lut();
        for z in zones {
            let id = z.area_id as usize;
            if id >= n || seen[id] {
                return Err(FrameError::AreaId(z.area_id));
            }
            seen[id] = true;
            let (r, nr, br) = channel(&z.r, p, &lut);
            let (g, ng, bg) = channel(&z.g, p, &lut);
            let (b, nb, bb) = channel(&z.b, p, &lut);
            f.color[id] = Rgb::new(r, g, b);
            f.black[id] = if nr == 0 || ng == 0 || nb == 0 { 0.0 } else { br.min(bg).min(bb) };
            f.count[id] = nr;
        }
        Ok(f)
    }

    pub fn zone_count(&self) -> usize {
        self.preset.zone_count()
    }
}

/// Mean (0..1), pixel count and black fraction of one channel's histogram.
fn channel(bins: &[u16; BINS], p: FrameParams, lut: &[f32; BINS]) -> (f32, u32, f32) {
    let mut n: u64 = 0;
    let mut sum = 0.0f64;
    let mut black: u64 = 0;
    for (i, &c) in bins.iter().enumerate() {
        n += u64::from(c);
        sum += f64::from(c) * f64::from(match p.mean {
            MeanKind::Code => i as f32 / 255.0,
            MeanKind::Linear => lut[i],
        });
        if i <= usize::from(p.black_code) {
            black += u64::from(c);
        }
    }
    if n == 0 {
        return (0.0, 0, 0.0);
    }
    let mean = (sum / n as f64) as f32;
    let mean = match p.mean {
        MeanKind::Code => mean,
        MeanKind::Linear => encode_srgb(mean),
    };
    (mean, n as u32, black as f32 / n as f32)
}

fn linear_lut() -> [f32; BINS] {
    let mut t = [0.0; BINS];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    }
    t
}

fn encode_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

/// One capture from the probe's `-d` file: header `VTH1`, preset, areas, t_ms (four little-endian u32), then the areas.
#[derive(Clone, Debug)]
pub struct DumpFrame {
    pub preset: Preset,
    pub t_ms: u32,
    pub zones: Vec<RawZone>,
}

pub fn parse_dump(bytes: &[u8]) -> Result<Vec<DumpFrame>, String> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < bytes.len() {
        let h = bytes.get(p..p + 16).ok_or("truncated header")?;
        let w = |i: usize| u32::from_le_bytes(h[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        if w(0) != 0x3148_5456 {
            return Err(format!("bad magic at {p}"));
        }
        let preset = u8::try_from(w(1)).ok().and_then(Preset::from_index).ok_or("bad preset")?;
        let n = w(2) as usize;
        if n != preset.zone_count() {
            return Err(format!("preset {} with {n} areas", w(1)));
        }
        p += 16;
        let mut zones = Vec::with_capacity(n);
        for _ in 0..n {
            let raw = bytes.get(p..p + RAW_ZONE_BYTES).ok_or("truncated area")?;
            zones.push(RawZone::from_bytes(raw).ok_or("short area")?);
            p += RAW_ZONE_BYTES;
        }
        out.push(DumpFrame { preset, t_ms: w(3), zones });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(id: u32, r: usize, g: usize, b: usize, count: u16) -> RawZone {
        let mut z = RawZone::empty(id);
        z.r[r] = count;
        z.g[g] = count;
        z.b[b] = count;
        z
    }

    #[test]
    fn preset_geometry_is_what_lgs_header_lists() {
        assert_eq!(Preset::Plain.zone_count(), 16);
        assert_eq!(Preset::TopBottom.zone_count(), 16);
        assert_eq!(Preset::SideBars.zone_count(), 16);
        assert_eq!(Preset::Games.zone_count(), 14);
        // Spot checks against the header text.
        assert_eq!(Preset::Plain.rects()[4], Rect::new(0.0, 0.0, 0.5, 1.0));
        assert_eq!(Preset::TopBottom.rects()[0], Rect::new(0.25, 0.1265, 0.75, 0.8735));
        assert_eq!(Preset::SideBars.rects()[8], Rect::new(0.5, 0.0, 0.87, 1.0));
        assert_eq!(Preset::Games.rects()[13], Rect::new(0.0, 0.62, 0.34, 1.0));
        for p in Preset::ALL {
            for r in p.rects() {
                assert!(r.x0 < r.x1 && r.y0 < r.y1 && r.x0 >= 0.0 && r.y0 >= 0.0 && r.x1 <= 1.0 && r.y1 <= 1.0, "{p:?} {r:?}");
            }
        }
        // The bar detector's strips are where it expects them in all three video presets.
        for p in [Preset::Plain, Preset::TopBottom, Preset::SideBars] {
            assert_eq!(p.rects()[TOP_STRIP], Rect::new(0.3, 0.0, 0.7, 0.1265));
            assert_eq!(p.rects()[LEFT_STRIP], Rect::new(0.0, 0.3, 0.13, 0.7));
        }
    }

    #[test]
    fn rect_distance() {
        let r = Rect::new(0.2, 0.2, 0.4, 0.4);
        assert_eq!(r.distance(0.3, 0.3), 0.0);
        assert!((r.distance(0.5, 0.3) - 0.1).abs() < 1e-6);
        assert!((r.distance(0.5, 0.5) - 0.1f32.hypot(0.1)).abs() < 1e-6);
    }

    #[test]
    fn code_mean_and_black_fraction() {
        let mut zones: Vec<RawZone> = (0..16).map(|i| flat(i, 128, 64, 255, 1000)).collect();
        // Area 3: half the pixels black, half code 200 in every channel.
        zones[3] = RawZone::empty(3);
        let z3 = &mut zones[3];
        for c in [&mut z3.r, &mut z3.g, &mut z3.b] {
            c[0] = 500;
            c[200] = 500;
        }
        let f = ZoneFrame::from_histograms(Preset::Plain, &zones, FrameParams::default()).unwrap();
        assert!((f.color[0].r - 128.0 / 255.0).abs() < 1e-6);
        assert!((f.color[0].g - 64.0 / 255.0).abs() < 1e-6);
        assert!((f.color[0].b - 1.0).abs() < 1e-6);
        assert_eq!(f.count[0], 1000);
        assert_eq!(f.black[0], 0.0);
        assert!((f.color[3].r - 100.0 / 255.0).abs() < 1e-5);
        assert!((f.black[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn linear_mean_is_pulled_up_by_highlights() {
        let mut z = RawZone::empty(0);
        for c in [&mut z.r, &mut z.g, &mut z.b] {
            c[0] = 500;
            c[255] = 500;
        }
        let mut zones = vec![z];
        zones.extend((1..16).map(RawZone::empty));
        let code = ZoneFrame::from_histograms(Preset::Plain, &zones, FrameParams::default()).unwrap();
        let lin = ZoneFrame::from_histograms(Preset::Plain, &zones, FrameParams { mean: MeanKind::Linear, ..FrameParams::default() }).unwrap();
        assert!((code.color[0].r - 0.5).abs() < 1e-6);
        // Half the light of white encodes to about 0.735 in sRGB.
        assert!((lin.color[0].r - 0.735).abs() < 0.01, "{}", lin.color[0].r);
    }

    #[test]
    fn empty_histogram_is_black_with_zero_count() {
        let zones: Vec<RawZone> = (0..16).map(RawZone::empty).collect();
        let f = ZoneFrame::from_histograms(Preset::Plain, &zones, FrameParams::default()).unwrap();
        assert_eq!(f.color[5], Rgb::BLACK);
        assert_eq!(f.count[5], 0);
        assert_eq!(f.black[5], 0.0);
    }

    #[test]
    fn wrong_area_count_or_id_is_an_error() {
        let zones: Vec<RawZone> = (0..16).map(RawZone::empty).collect();
        assert_eq!(
            ZoneFrame::from_histograms(Preset::Games, &zones, FrameParams::default()),
            Err(FrameError::ZoneCount { expected: 14, got: 16 })
        );
        let mut dup = zones.clone();
        dup[4].area_id = 3;
        assert_eq!(ZoneFrame::from_histograms(Preset::Plain, &dup, FrameParams::default()), Err(FrameError::AreaId(3)));
        let mut high = zones;
        high[15].area_id = 16;
        assert_eq!(ZoneFrame::from_histograms(Preset::Plain, &high, FrameParams::default()), Err(FrameError::AreaId(16)));
    }

    #[test]
    fn raw_zone_and_dump_round_trip() {
        let mut bytes = Vec::new();
        for v in [0x3148_5456u32, 3, 14, 1234] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for id in 0..14u32 {
            bytes.extend_from_slice(&id.to_le_bytes());
            for ch in 0..3u16 {
                for bin in 0..BINS as u16 {
                    let v = bin.wrapping_mul(7).wrapping_add(ch).wrapping_add(id as u16);
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        assert_eq!(bytes.len(), 16 + 14 * RAW_ZONE_BYTES);
        let frames = parse_dump(&bytes).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].preset, Preset::Games);
        assert_eq!(frames[0].t_ms, 1234);
        assert_eq!(frames[0].zones[13].area_id, 13);
        assert_eq!(frames[0].zones[2].g[10], 10u16.wrapping_mul(7).wrapping_add(1).wrapping_add(2));
        assert!(parse_dump(&bytes[..bytes.len() - 1]).is_err());
        assert!(parse_dump(&[0u8; 16]).is_err());
    }
}
