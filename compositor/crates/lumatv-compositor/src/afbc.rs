//! The scan-out pages as ARM AFBC (`--afbc`), and the page layouts the primary plane can be given (`--page-alpha`).
//!
//! The OSD reads the primary plane's page every refresh: 1920x1080x4 bytes is 8.29 MB, 498 MB/s at 60 Hz and 995 MB/s at 120. The plane lists AFBC
//! `0x0800000000000062` (32x8 superblocks, split, sparse) for AB24 and XB24, and LG's own compositor scans such buffers out (its retained outputs
//! on the set are AB24 `...62`, 8,486,912 bytes, `docs/research/scanout-research.md` E3). This module holds the arithmetic and the decisions around
//! it; the pages themselves are made in `kms`. What is measured and what is not is in `docs/research/hw-planes-result.md`.

use serde_json::{Value, json};

use crate::buffer::{fourcc, modifier};

/// `--afbc`: no AFBC (the default), only ask what the display and the GPU driver list, or make the pages AFBC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    Probe,
    On,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "off" => Some(Mode::Off),
            "probe" => Some(Mode::Probe),
            "on" => Some(Mode::On),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Probe => "probe",
            Mode::On => "on",
        }
    }
}

/// What the primary plane's pages are. XR24 is what the compositor has always made: the display driver reads such a layer with a constant alpha
/// of 0xff (`osd_drm_flip_image`: "setup global alpha if pixel format is xRGB"; the live `fb0/status` reads `global_alpha_en 1` under LumaTV's
/// compositor and 0 under LG's AB24 pages), so a hole cleared to transparent is opaque black on the panel. AR24 carries the alpha the frame
/// already has. AFBC pages are AB24: the plane lists AFBC for the ABGR order only (`lg1k_plane_format_mod_supported`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKind {
    Xrgb,
    Argb,
    Afbc,
}

impl PageKind {
    pub fn fourcc(self) -> u32 {
        match self {
            PageKind::Xrgb => fourcc::XRGB8888,
            PageKind::Argb => fourcc::ARGB8888,
            PageKind::Afbc => fourcc::ABGR8888,
        }
    }

    pub fn modifier(self) -> u64 {
        match self {
            PageKind::Afbc => modifier::AFBC_32X8_SPLIT_SPARSE,
            _ => modifier::LINEAR,
        }
    }

    /// Whether a pixel's alpha reaches the display (a hole shows the video plane).
    pub fn has_alpha(self) -> bool {
        self != PageKind::Xrgb
    }

    pub fn name(self) -> &'static str {
        match self {
            PageKind::Xrgb => "XR24 linear",
            PageKind::Argb => "AR24 linear",
            PageKind::Afbc => "AB24 AFBC 32x8 split sparse",
        }
    }

    /// The kind the switches ask for: AFBC first, else linear with alpha when holes need it, else the original.
    pub fn wanted(afbc: Mode, page_alpha: bool) -> PageKind {
        match (afbc, page_alpha) {
            (Mode::On, _) => PageKind::Afbc,
            (_, true) => PageKind::Argb,
            _ => PageKind::Xrgb,
        }
    }

    /// The kind to fall back to when AFBC cannot be had.
    pub fn linear(page_alpha: bool) -> PageKind {
        if page_alpha { PageKind::Argb } else { PageKind::Xrgb }
    }

    /// Whether a video hole on pages of this kind is opaque black on the panel (the display driver reads an XR24 layer with a constant alpha).
    pub fn video_holes_opaque(self) -> bool {
        !self.has_alpha()
    }

    /// The kinds a ring is tried in when `self` is asked for: each one down to the original XR24, which every plane takes. A ring that cannot be
    /// made with alpha must still start (the screen comes up), with its video holes opaque, and the report says so.
    pub fn fallbacks(self, page_alpha: bool) -> Vec<PageKind> {
        let mut chain = vec![self];
        if self == PageKind::Afbc {
            chain.push(PageKind::linear(page_alpha));
        }
        if chain.last() != Some(&PageKind::Xrgb) {
            chain.push(PageKind::Xrgb);
        }
        chain.dedup();
        chain
    }
}

/// What [`first_ring`] made, and what it passed over.
#[derive(Debug)]
pub struct RingMade<T> {
    pub ring: T,
    pub kind: PageKind,
    /// The kinds tried first and why each failed, in order.
    pub failed: Vec<(PageKind, String)>,
}

/// Makes a ring of the first of `chain` that `make` can: one that fails is logged and the next is tried; the last failure is the error when none can.
pub fn first_ring<T>(chain: &[PageKind], mut make: impl FnMut(PageKind) -> std::io::Result<T>) -> std::io::Result<RingMade<T>> {
    let mut failed = Vec::new();
    let mut last = None;
    for &kind in chain {
        match make(kind) {
            Ok(ring) => return Ok(RingMade { ring, kind, failed }),
            Err(e) => {
                failed.push((kind, e.to_string()));
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no page kind to try")))
}

/// A superblock of this modifier is 32x8 pixels. Each has a header of [`HEADER_BYTES`] and a body of [`BODY_BYTES`]; `sparse` places each body at its
/// own full-size slot, so a compressed picture is no smaller in memory, only in the bytes read and written.
pub const SUPERBLOCK_W: u32 = 32;
pub const SUPERBLOCK_H: u32 = 8;
pub const HEADER_BYTES: u32 = 16;
/// 32x8 pixels of 4 bytes.
pub const BODY_BYTES: u32 = SUPERBLOCK_W * SUPERBLOCK_H * 4;

fn align(v: u64, to: u64) -> u64 {
    v.div_ceil(to) * to
}

/// Superblocks across and down for a `width` by `height` picture: the height is taken up to a multiple of 16 rows, as the GPU's allocator does (1080 is
/// 1088), which is what reproduces the two sizes seen on the set.
pub fn superblocks(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(SUPERBLOCK_W), (align(u64::from(height), 16) as u32).div_ceil(SUPERBLOCK_H))
}

/// The memory an AFBC `...62` buffer of this size takes: the headers, then the bodies, in whole pages. 1920x1080 is 8,486,912 bytes and 768x2160 is
/// 6,742,016, the two sizes of LG's output buffers in the DRM state read on the set.
pub fn buffer_size(width: u32, height: u32) -> usize {
    let (cols, rows) = superblocks(width, height);
    align(u64::from(cols) * u64::from(rows) * u64::from(HEADER_BYTES + BODY_BYTES), 4096) as usize
}

/// The headers' share of [`buffer_size`].
pub fn header_bytes(width: u32, height: u32) -> usize {
    let (cols, rows) = superblocks(width, height);
    cols as usize * rows as usize * HEADER_BYTES as usize
}

/// Rows of a dumb buffer `pitch` bytes wide that hold an AFBC buffer of `width` by `height`: the allocation is made as an ordinary buffer of
/// this many rows, and the framebuffer is made over it with the picture's own size and a pitch of `width * 4` (what the driver's AFBC format
/// info takes, `lg1k_get_format_info`).
pub fn dumb_rows(width: u32, height: u32, pitch: u32) -> u32 {
    (buffer_size(width, height) as u32).div_ceil(pitch.max(1))
}

/// What a modifier is, for the report: `ARM AFBC 32x8 split sparse`.
pub fn describe(m: u64) -> String {
    if m == modifier::LINEAR {
        return "linear".into();
    }
    let (vendor, kind, mode) = ((m >> 56) as u8, ((m >> 52) & 0xf) as u8, m & 0x000f_ffff_ffff_ffff);
    if vendor != 0x08 {
        return format!("vendor {vendor:#x} modifier {m:#x}");
    }
    let what = match kind {
        0 => "AFBC",
        1 => "AFRC",
        _ => return format!("ARM type {kind} modifier {m:#x}"),
    };
    if kind == 1 {
        return format!("ARM {what} {mode:#x}");
    }
    let block = match mode & 0xf {
        1 => "16x16",
        2 => "32x8",
        3 => "64x4",
        4 => "32x8+64x4",
        _ => "block ?",
    };
    let flags: Vec<&str> = [
        (1 << 4, "ytr"),
        (1 << 5, "split"),
        (1 << 6, "sparse"),
        (1 << 7, "cbr"),
        (1 << 8, "tiled"),
        (1 << 9, "sc"),
        (1 << 10, "db"),
        (1 << 11, "bch"),
        (1 << 12, "usm"),
    ]
    .iter()
    .filter(|(bit, _)| mode & bit != 0)
    .map(|(_, n)| *n)
    .collect();
    format!("ARM {what} {block} {}", flags.join(" "))
}

/// What the display reads of the page in a second, in MB/s (10^6 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub linear: f64,
    pub afbc: f64,
}

impl Reading {
    pub fn saved(&self) -> f64 {
        self.linear - self.afbc
    }

    pub fn saved_fraction(&self) -> f64 {
        self.saved() / self.linear
    }
}

/// The scan-out's read of a `width` by `height` page at `hz`: linear is every byte of every row each refresh; AFBC is the headers plus `body_ratio` of the
/// bodies. `body_ratio` is not measured: it is the compressed share of the pixels' bytes, which depends on the picture.
pub fn scanout_reading(width: u32, height: u32, hz: f64, body_ratio: f64) -> Reading {
    let linear = f64::from(width) * f64::from(height) * 4.0 * hz / 1e6;
    let (cols, rows) = superblocks(width, height);
    let bodies = f64::from(cols) * f64::from(rows) * f64::from(BODY_BYTES);
    let afbc = (header_bytes(width, height) as f64 + bodies * body_ratio) * hz / 1e6;
    Reading { linear, afbc }
}

/// The body ratios the estimate is shown at, and what each stands for. A transparent block (a video hole) is a solid-colour superblock that AFBC stores
/// in its header and a few bytes; a flat or gradient Home about a quarter; text and icons on glass about half; a photograph or noise most of it.
pub const SCENARIOS: [(&str, f64); 3] = [("Home, mostly flat or a hole", 0.25), ("typical UI", 0.5), ("photograph or noise", 0.85)];

/// `scanout_reading` for each scenario at 60 and 120 Hz.
pub fn estimate(width: u32, height: u32) -> Value {
    let row = |hz: f64| {
        SCENARIOS
            .iter()
            .map(|(what, ratio)| {
                let r = scanout_reading(width, height, hz, *ratio);
                json!({"content": what, "bodyRatio": ratio, "linearMBs": round1(r.linear), "afbcMBs": round1(r.afbc), "savedMBs": round1(r.saved()), "savedPercent": round1(r.saved_fraction() * 100.0)})
            })
            .collect::<Vec<_>>()
    };
    json!({
        "pageBytesLinear": u64::from(width) * u64::from(height) * 4,
        "pageBytesAfbc": buffer_size(width, height),
        "headerBytes": header_bytes(width, height),
        "hz60": row(60.0),
        "hz120": row(120.0),
        "note": "estimates: the body ratios are assumptions about the picture, not readings of this panel; the memory a page takes does not shrink (sparse layout)",
    })
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// A modifier the GPU driver lists for a format, and whether the format with it can only be sampled (an image bound to a texture) and not drawn into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EglModifier {
    pub fourcc: u32,
    pub modifier: u64,
    pub external_only: bool,
}

/// What `--afbc probe` and `--afbc on` learn, without a picture on the screen: what the plane lists and what the driver lists.
#[derive(Clone, Debug, Default)]
pub struct Probe {
    /// The primary plane's `(fourcc, modifier)` pairs with a modifier other than linear.
    pub plane: Vec<(u32, u64)>,
    /// `Some` once the GPU driver was asked (`eglQueryDmaBufModifiersEXT`); `None` when it has no such call.
    pub egl: Option<Vec<EglModifier>>,
}

impl Probe {
    pub fn plane_takes(&self) -> bool {
        self.plane.contains(&(fourcc::ABGR8888, modifier::AFBC_32X8_SPLIT_SPARSE))
    }

    /// Whether the driver lists the layout for ABGR8888 as one it can draw into: `None` when the driver was not asked or lists nothing.
    pub fn gpu_draws(&self) -> Option<bool> {
        let list = self.egl.as_ref()?;
        Some(list.iter().any(|m| m.fourcc == fourcc::ABGR8888 && m.modifier == modifier::AFBC_32X8_SPLIT_SPARSE && !m.external_only))
    }

    /// The answer in a line.
    pub fn verdict(&self) -> &'static str {
        match (self.plane_takes(), self.gpu_draws()) {
            (false, _) => "the plane does not list AFBC for AB24: no",
            (true, Some(false)) => "the plane takes it and the GPU driver does not list it to draw into: no",
            (true, None) => "the plane takes it; the GPU driver was not asked (no modifier query): try it",
            (true, Some(true)) => "the plane takes it and the GPU driver lists it to draw into: yes, subject to the picture being right on the panel",
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "plane": self.plane.iter().map(|(f, m)| json!({"format": fourcc::name(*f), "modifier": format!("{m:#018x}"), "is": describe(*m)})).collect::<Vec<_>>(),
            "egl": self.egl.as_ref().map(|l| l.iter().map(|m| json!({"format": fourcc::name(m.fourcc), "modifier": format!("{:#018x}", m.modifier), "is": describe(m.modifier), "externalOnly": m.external_only})).collect::<Vec<_>>()),
            "planeTakes": self.plane_takes(),
            "gpuDraws": self.gpu_draws(),
            "verdict": self.verdict(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_reproduces_the_two_sizes_of_lgs_buffers_on_the_set() {
        // DRM state read on the set: surface-manager's AB24 `...62` output 1920x1080 (8,486,912 bytes) and its 768x2160 secondary (6,742,016)
        assert_eq!(buffer_size(1920, 1080), 8_486_912);
        assert_eq!(buffer_size(768, 2160), 6_742_016);
        assert_eq!(superblocks(1920, 1080), (60, 136), "1080 is taken up to 1088 rows");
        assert_eq!(header_bytes(1920, 1080), 130_560);
        assert_eq!(buffer_size(1920, 1080) % 4096, 0);
    }

    #[test]
    fn a_dumb_buffer_of_the_pages_pitch_holds_the_buffer_with_less_than_a_row_to_spare() {
        let rows = dumb_rows(1920, 1080, 7680);
        assert_eq!(rows, 1106);
        let held = u64::from(rows) * 7680;
        assert!(held >= buffer_size(1920, 1080) as u64);
        assert!(held - (buffer_size(1920, 1080) as u64) < 7680);
    }

    #[test]
    fn modifiers_are_named() {
        assert_eq!(describe(modifier::LINEAR), "linear");
        assert_eq!(describe(modifier::AFBC_32X8_SPLIT_SPARSE), "ARM AFBC 32x8 split sparse");
        assert_eq!(describe(modifier::AFBC_16X16_YTR_SPARSE_TILED), "ARM AFBC 16x16 ytr sparse tiled");
        assert!(describe(0x0100_0000_0000_0001).contains("vendor 0x1"));
        assert!(describe(0x0810_0000_0000_0000 | 3).contains("AFRC"));
    }

    #[test]
    fn the_page_kinds_say_what_they_are_and_the_switches_pick_one() {
        assert_eq!(PageKind::Xrgb.fourcc(), fourcc::XRGB8888);
        assert_eq!(PageKind::Argb.fourcc(), fourcc::ARGB8888);
        assert_eq!((PageKind::Afbc.fourcc(), PageKind::Afbc.modifier()), (fourcc::ABGR8888, 0x0800_0000_0000_0062));
        assert!(!PageKind::Xrgb.has_alpha() && PageKind::Argb.has_alpha() && PageKind::Afbc.has_alpha());
        assert_eq!(PageKind::wanted(Mode::Off, false), PageKind::Xrgb, "the opt-out: as it has always been");
        assert_eq!(PageKind::wanted(Mode::Off, true), PageKind::Argb);
        assert_eq!(PageKind::wanted(Mode::Probe, false), PageKind::Xrgb, "a probe changes nothing");
        assert_eq!(PageKind::wanted(Mode::On, false), PageKind::Afbc);
        assert_eq!(PageKind::wanted(Mode::On, true), PageKind::Afbc);
        assert_eq!((PageKind::linear(true), PageKind::linear(false)), (PageKind::Argb, PageKind::Xrgb));
    }

    #[test]
    fn a_ring_that_cannot_have_alpha_falls_back_to_xr24_and_says_its_holes_are_opaque() {
        use PageKind::*;
        assert_eq!(Argb.fallbacks(true), [Argb, Xrgb]);
        assert_eq!(Xrgb.fallbacks(false), [Xrgb]);
        assert_eq!(Afbc.fallbacks(true), [Afbc, Argb, Xrgb]);
        assert_eq!(Afbc.fallbacks(false), [Afbc, Xrgb]);
        assert!(Xrgb.video_holes_opaque() && !Argb.video_holes_opaque() && !Afbc.video_holes_opaque());
        // the memory for AR24 pages is not there (CMA short): the screen still comes up on XR24
        let mut tried = Vec::new();
        let made = first_ring(&Argb.fallbacks(true), |k| {
            tried.push(k);
            if k == Argb { Err(std::io::Error::other("out of CMA")) } else { Ok(3usize) }
        })
        .unwrap();
        assert_eq!((made.kind, made.ring, tried), (Xrgb, 3, vec![Argb, Xrgb]));
        assert_eq!(made.failed.len(), 1);
        assert!(made.failed[0].1.contains("out of CMA") && made.kind.video_holes_opaque());
        // the first that works is the one taken
        let made = first_ring(&Afbc.fallbacks(true), |k| if k == Afbc { Err(std::io::Error::other("refused")) } else { Ok(()) }).unwrap();
        assert_eq!(made.kind, Argb);
        // nothing works: the last error
        let e = first_ring::<()>(&Argb.fallbacks(true), |k| Err(std::io::Error::other(format!("no {}", k.name())))).unwrap_err();
        assert!(e.to_string().contains("XR24"), "{e}");
    }

    #[test]
    fn the_scanout_reading_is_the_pages_bytes_times_the_refresh() {
        let r = scanout_reading(1920, 1080, 60.0, 1.0);
        assert!((r.linear - 497.664).abs() < 1e-9, "{r:?}");
        // an incompressible picture reads the headers on top of the pixels, and its 1088 rows of superblocks for 1080 of picture
        assert!((r.afbc - (130_560.0 + 8_355_840.0) * 60.0 / 1e6).abs() < 1e-9 && r.afbc > r.linear, "{r:?}");
        let half = scanout_reading(1920, 1080, 60.0, 0.5);
        assert!((half.afbc - 258.5).abs() < 0.1, "{half:?}");
        assert!((half.saved() - 239.2).abs() < 0.2 && (half.saved_fraction() - 0.48).abs() < 0.01);
        let twice = scanout_reading(1920, 1080, 120.0, 0.5);
        assert!((twice.afbc - 2.0 * half.afbc).abs() < 1e-9 && (twice.linear - 995.328).abs() < 1e-9);
    }

    #[test]
    fn the_estimate_lists_each_scenario_at_both_rates_and_says_it_is_one() {
        let e = estimate(1920, 1080);
        assert_eq!(e["pageBytesAfbc"], 8_486_912);
        assert_eq!(e["pageBytesLinear"], 8_294_400);
        for key in ["hz60", "hz120"] {
            let rows = e[key].as_array().unwrap();
            assert_eq!(rows.len(), SCENARIOS.len());
            let saved: Vec<f64> = rows.iter().map(|r| r["savedMBs"].as_f64().unwrap()).collect();
            assert!(saved[0] > saved[1] && saved[1] > saved[2], "the more compressible, the more saved: {saved:?}");
            assert!(saved[2] > 0.0, "even a photograph's body ratio of 0.85 saves a little: {saved:?}");
        }
        assert!(e["note"].as_str().unwrap().contains("estimates"));
    }

    #[test]
    fn the_probe_answers_in_the_order_the_questions_come() {
        let mut p = Probe::default();
        assert_eq!(p.verdict(), "the plane does not list AFBC for AB24: no");
        p.plane.push((fourcc::ABGR8888, modifier::AFBC_32X8_SPLIT_SPARSE));
        assert!(p.plane_takes() && p.gpu_draws().is_none());
        assert!(p.verdict().contains("try it"));
        p.egl = Some(vec![EglModifier { fourcc: fourcc::ABGR8888, modifier: modifier::LINEAR, external_only: false }]);
        assert_eq!(p.gpu_draws(), Some(false));
        assert!(p.verdict().contains("does not list it to draw into"));
        p.egl = Some(vec![EglModifier { fourcc: fourcc::ABGR8888, modifier: modifier::AFBC_32X8_SPLIT_SPARSE, external_only: true }]);
        assert_eq!(p.gpu_draws(), Some(false), "sampling is not drawing");
        p.egl = Some(vec![EglModifier { fourcc: fourcc::ABGR8888, modifier: modifier::AFBC_32X8_SPLIT_SPARSE, external_only: false }]);
        assert_eq!(p.gpu_draws(), Some(true));
        assert!(p.verdict().starts_with("the plane takes it and the GPU driver lists it"));
        let j = p.json();
        assert_eq!(j["planeTakes"], true);
        assert_eq!(j["plane"][0]["is"], "ARM AFBC 32x8 split sparse");
    }

    #[test]
    fn the_mode_parses_and_is_off_by_default() {
        assert_eq!(Mode::default(), Mode::Off);
        assert_eq!((Mode::parse("on"), Mode::parse("probe"), Mode::parse("off"), Mode::parse("1")), (Some(Mode::On), Some(Mode::Probe), Some(Mode::Off), None));
    }
}
