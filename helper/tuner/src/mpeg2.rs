//! The smallest MPEG-2 video encoder a slate needs (ISO/IEC 13818-2, Main Profile, 4:2:0, frame pictures): a still picture as an intra
//! picture, and a P picture whose macroblocks are all skipped, which repeats the last one in a few hundred bytes. A slate is mostly flat
//! colour: a block whose 64 samples are equal costs its DC alone, and only the edges of the words and the bar go through the transform.
//!
//! The picture's size, rate and shape are the channel's own when they are known ([`Format::of_sequence`]), so the decoder downstream
//! sees the slate and the programme as one sequence. Every slice is a macroblock row and starts its own predictions, so a row that did
//! not change keeps its bytes ([`Encoder::intra_slice`]).

use std::sync::LazyLock;

/// Frame rates by `frame_rate_code` (13818-2 table 6-4): numerator and denominator.
const RATES: [(u64, u64); 9] = [
    (0, 1),
    (24_000, 1001),
    (24, 1),
    (25, 1),
    (30_000, 1001),
    (30, 1),
    (50, 1),
    (60_000, 1001),
    (60, 1),
];

const SEQUENCE_HEADER: u8 = 0xB3;
const EXTENSION: u8 = 0xB5;
const GOP: u8 = 0xB8;
const PICTURE: u8 = 0x00;

/// What a picture is: its size, its rate and its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub width: u16,
    pub height: u16,
    /// `frame_rate_code`: 4 is 30000/1001, 7 is 60000/1001.
    pub rate: u8,
    /// `aspect_ratio_information`: 2 is 4:3, 3 is 16:9.
    pub aspect: u8,
    pub progressive: bool,
}

impl Format {
    /// A channel not seen yet: ATSC's 1080i, the shape of most main channels.
    pub const FALLBACK: Self = Self {
        width: 1920,
        height: 1080,
        rate: 4,
        aspect: 3,
        progressive: false,
    };

    /// The format a sequence header gives, and the sequence extension right after it (progressive or interlaced); `es` is the elementary
    /// stream from anywhere before the header's start code. Only what a slate can copy: a size of at most 1920x1088 and a rate of 13818-2.
    pub fn of_sequence(es: &[u8]) -> Option<Self> {
        let at = find_code(es, SEQUENCE_HEADER)?;
        let h = es.get(at + 4..at + 12)?;
        let width = (u16::from(h[0]) << 4) | u16::from(h[1] >> 4);
        let height = (u16::from(h[1] & 0x0F) << 8) | u16::from(h[2]);
        let (aspect, rate) = (h[3] >> 4, h[3] & 0x0F);
        if !(1..=8).contains(&rate) || !(1..=4).contains(&aspect) || width == 0 || width > 1920 || height == 0 || height > 1088 {
            return None;
        }
        // the extension follows the header and its two optional matrices; the first extension start code after it is the sequence's
        let rest = es.get(at + 12..)?;
        let progressive = find_code(rest, EXTENSION)
            .and_then(|e| rest.get(e + 4..e + 6))
            .filter(|x| x[0] >> 4 == 1)
            .is_some_and(|x| x[1] & 0x08 != 0);
        Some(Self {
            width,
            height,
            rate,
            aspect,
            progressive,
        })
    }

    /// The 90 kHz time of frame `n` after the first.
    pub fn ticks(&self, n: u64) -> u64 {
        let (num, den) = RATES[usize::from(self.rate)];
        n * 90_000 * den / num
    }

    /// Frames a second, near enough to space them by.
    pub fn fps(&self) -> f64 {
        let (num, den) = RATES[usize::from(self.rate)];
        num as f64 / den as f64
    }

    pub fn mb_width(&self) -> usize {
        usize::from(self.width).div_ceil(16)
    }

    /// Macroblock rows: an interlaced sequence's frame is a whole number of field pairs of macroblocks (13818-2 6.3.3).
    pub fn mb_height(&self) -> usize {
        let h = usize::from(self.height);
        if self.progressive { h.div_ceil(16) } else { 2 * h.div_ceil(32) }
    }

    /// Whether this is high definition: BT.709 colours, Main Profile at High Level.
    pub fn hd(&self) -> bool {
        self.width > 720 || self.height > 576
    }

    /// How wide a square-pixel picture of the same height would be: what a coded pixel spans on the screen.
    pub fn display_width(&self) -> f32 {
        let h = f32::from(self.height);
        match self.aspect {
            2 => h * 4.0 / 3.0,
            3 => h * 16.0 / 9.0,
            4 => h * 2.21,
            _ => f32::from(self.width),
        }
    }
}

/// The first start code `00 00 01 code` in `es`: where its first zero is.
fn find_code(es: &[u8], code: u8) -> Option<usize> {
    es.windows(4).position(|w| w == [0, 0, 1, code])
}

/// A picture's samples, 4:2:0, macroblock-aligned: the luma `width` x `height`, each chroma plane half of that each way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planes {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
}

impl Planes {
    /// A picture of `format` in one colour (Y, Cb, Cr).
    pub fn new(format: &Format, colour: [u8; 3]) -> Self {
        let (width, height) = (format.mb_width() * 16, format.mb_height() * 16);
        Self {
            width,
            height,
            y: vec![colour[0]; width * height],
            cb: vec![colour[1]; width * height / 4],
            cr: vec![colour[2]; width * height / 4],
        }
    }
}

/// Writes bits most significant first.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, len: u32, value: u32) {
        debug_assert!(len <= 32 && (len == 32 || value >> len == 0));
        self.acc = (self.acc << len) | u64::from(value);
        self.n += len;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1 << self.n) - 1;
    }

    /// Zero bits to the next byte, then the start code.
    fn start(&mut self, code: u8) {
        self.align();
        self.out.extend_from_slice(&[0, 0, 1, code]);
    }

    fn align(&mut self) {
        if self.n > 0 {
            self.put(8 - self.n, 0);
        }
    }

    fn done(mut self) -> Vec<u8> {
        self.align();
        self.out
    }
}

/// `dct_dc_size_luminance` and `dct_dc_size_chrominance` (13818-2 tables B.12, B.13): code and length by size.
const DC_LUMA: [(u32, u32); 12] = [
    (0x4, 3),
    (0x0, 2),
    (0x1, 2),
    (0x5, 3),
    (0x6, 3),
    (0xE, 4),
    (0x1E, 5),
    (0x3E, 6),
    (0x7E, 7),
    (0xFE, 8),
    (0x1FE, 9),
    (0x1FF, 9),
];
const DC_CHROMA: [(u32, u32); 12] = [
    (0x0, 2),
    (0x1, 2),
    (0x2, 2),
    (0x6, 3),
    (0xE, 4),
    (0x1E, 5),
    (0x3E, 6),
    (0x7E, 7),
    (0xFE, 8),
    (0x1FE, 9),
    (0x3FE, 10),
    (0x3FF, 10),
];

/// Table B.14 (`intra_vlc_format` 0), the codes of (run, level) before their sign bit, in this order: run 0 with levels 1 to 40, run 1
/// with 1 to 18, then runs 2 to 31 with as many levels as [`LEVELS_OF_RUN`] says.
const AC: [(u16, u8); 111] = [
    (0x3, 2), (0x4, 4), (0x5, 5), (0x6, 7), (0x26, 8), (0x21, 8), (0xa, 10), (0x1d, 12),
    (0x18, 12), (0x13, 12), (0x10, 12), (0x1a, 13), (0x19, 13), (0x18, 13), (0x17, 13), (0x1f, 14),
    (0x1e, 14), (0x1d, 14), (0x1c, 14), (0x1b, 14), (0x1a, 14), (0x19, 14), (0x18, 14), (0x17, 14),
    (0x16, 14), (0x15, 14), (0x14, 14), (0x13, 14), (0x12, 14), (0x11, 14), (0x10, 14), (0x18, 15),
    (0x17, 15), (0x16, 15), (0x15, 15), (0x14, 15), (0x13, 15), (0x12, 15), (0x11, 15), (0x10, 15),
    (0x3, 3), (0x6, 6), (0x25, 8), (0xc, 10), (0x1b, 12), (0x16, 13), (0x15, 13), (0x1f, 15),
    (0x1e, 15), (0x1d, 15), (0x1c, 15), (0x1b, 15), (0x1a, 15), (0x19, 15), (0x13, 16), (0x12, 16),
    (0x11, 16), (0x10, 16), (0x5, 4), (0x4, 7), (0xb, 10), (0x14, 12), (0x14, 13), (0x7, 5),
    (0x24, 8), (0x1c, 12), (0x13, 13), (0x6, 5), (0xf, 10), (0x12, 12), (0x7, 6), (0x9, 10),
    (0x12, 13), (0x5, 6), (0x1e, 12), (0x14, 16), (0x4, 6), (0x15, 12), (0x7, 7), (0x11, 12),
    (0x5, 7), (0x11, 13), (0x27, 8), (0x10, 13), (0x23, 8), (0x1a, 16), (0x22, 8), (0x19, 16),
    (0x20, 8), (0x18, 16), (0xe, 10), (0x17, 16), (0xd, 10), (0x16, 16), (0x8, 10), (0x15, 16),
    (0x1f, 12), (0x1a, 12), (0x19, 12), (0x17, 12), (0x16, 12), (0x1f, 13), (0x1e, 13), (0x1d, 13),
    (0x1c, 13), (0x1b, 13), (0x1f, 16), (0x1e, 16), (0x1d, 16), (0x1c, 16), (0x1b, 16),
];
/// How many levels table B.14 has a code for, by run.
const LEVELS_OF_RUN: [u8; 32] = [40, 18, 5, 4, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
const ESCAPE: (u32, u32) = (0x1, 6);
const END_OF_BLOCK: (u32, u32) = (0x2, 2);

/// `macroblock_address_increment` 1 to 33 (table B.1), and the escape that adds 33.
const ADDRESS: [(u32, u32); 33] = [
    (0x1, 1), (0x3, 3), (0x2, 3), (0x3, 4), (0x2, 4), (0x3, 5), (0x2, 5), (0x7, 7), (0x6, 7), (0xb, 8), (0xa, 8), (0x9, 8),
    (0x8, 8), (0x7, 8), (0x6, 8), (0x17, 10), (0x16, 10), (0x15, 10), (0x14, 10), (0x13, 10), (0x12, 10), (0x23, 11), (0x22, 11),
    (0x21, 11), (0x20, 11), (0x1f, 11), (0x1e, 11), (0x1d, 11), (0x1c, 11), (0x1b, 11), (0x1a, 11), (0x19, 11), (0x18, 11),
];
const ADDRESS_ESCAPE: (u32, u32) = (0x8, 11);

/// The default intra quantiser matrix, row by row.
const INTRA_MATRIX: [u16; 64] = [
    8, 16, 19, 22, 26, 27, 29, 34, //
    16, 16, 22, 24, 27, 29, 34, 37, //
    19, 22, 26, 27, 29, 34, 34, 38, //
    22, 22, 26, 27, 29, 34, 37, 40, //
    22, 26, 27, 29, 32, 35, 40, 48, //
    26, 27, 29, 32, 35, 40, 48, 58, //
    26, 27, 29, 34, 38, 46, 56, 69, //
    27, 29, 35, 38, 46, 56, 69, 83,
];

/// The zigzag scan: the raster index of each coefficient in transmission order.
const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56,
    57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// `quantiser_scale_code` of the intra slices: 3 is a scale of 6 (`q_scale_type` 0), fine enough that the words keep their edges.
const QUANT: u32 = 3;

fn put_address(bits: &mut Bits, mut increment: usize) {
    while increment > 33 {
        bits.put(ADDRESS_ESCAPE.1, ADDRESS_ESCAPE.0);
        increment -= 33;
    }
    let (code, len) = ADDRESS[increment - 1];
    bits.put(len, code);
}

/// The forward DCT of 13818-2 annex A, scaled so the DC of a flat block is eight times its sample.
fn dct(block: &[f32; 64]) -> [f32; 64] {
    let cos = &*COS;
    let mut rows = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            rows[y * 8 + u] = (0..8).map(|x| block[y * 8 + x] * cos[u * 8 + x]).sum();
        }
    }
    let mut out = [0f32; 64];
    for v in 0..8 {
        for u in 0..8 {
            out[v * 8 + u] = (0..8).map(|y| rows[y * 8 + u] * cos[v * 8 + y]).sum();
        }
    }
    out
}

/// `C(u)/2 * cos((2x+1) u pi / 16)`, by `u * 8 + x`.
static COS: LazyLock<[f32; 64]> = LazyLock::new(|| {
    let mut t = [0f32; 64];
    for u in 0..8 {
        let c = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
        for x in 0..8 {
            t[u * 8 + x] = 0.5 * c * (((2 * x + 1) as f32) * (u as f32) * std::f32::consts::PI / 16.0).cos();
        }
    }
    t
});

/// Encodes still pictures of one format.
#[derive(Clone, Debug)]
pub struct Encoder {
    format: Format,
}

impl Encoder {
    pub fn new(format: Format) -> Self {
        Self { format }
    }

    pub fn format(&self) -> &Format {
        &self.format
    }

    /// The slice of macroblock row `row` of an intra picture of `planes`: its start code to its last bit, padded to the byte.
    pub fn intra_slice(&self, planes: &Planes, row: usize) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.start(row as u8 + 1);
        bits.put(5, QUANT);
        bits.put(1, 0);
        let mut predict = [128i32; 3];
        for col in 0..self.format.mb_width() {
            // the first macroblock's increment is its column plus one; the rest follow one another
            bits.put(1, 1);
            // macroblock_type: intra (table B.2)
            bits.put(1, 1);
            let (x, y) = (col * 16, row * 16);
            for (bx, by) in [(x, y), (x + 8, y), (x, y + 8), (x + 8, y + 8)] {
                block(&mut bits, &planes.y, planes.width, bx, by, &mut predict[0], &DC_LUMA);
            }
            let cw = planes.width / 2;
            block(&mut bits, &planes.cb, cw, x / 2, y / 2, &mut predict[1], &DC_CHROMA);
            block(&mut bits, &planes.cr, cw, x / 2, y / 2, &mut predict[2], &DC_CHROMA);
        }
        bits.done()
    }

    /// An intra picture that starts a sequence and a closed GOP, so a decoder can start on it: `slices` are its rows, in order, from
    /// [`Encoder::intra_slice`].
    pub fn intra(&self, slices: &[Vec<u8>]) -> Vec<u8> {
        let mut bits = Bits::default();
        self.sequence(&mut bits);
        bits.start(GOP);
        // time code 00:00:00:00 with its marker bit, closed, not broken
        bits.put(25, 1 << 12);
        bits.put(1, 1);
        bits.put(1, 0);
        self.picture(&mut bits, 0, 1);
        let mut out = bits.done();
        for s in slices {
            out.extend_from_slice(s);
        }
        out
    }

    /// A P picture in which every macroblock is skipped: the last picture shown again. Each row's slice codes only its first and last
    /// macroblock (which may not be skipped), forward-predicted with a zero vector and nothing coded.
    pub fn repeat(&self, temporal_reference: u16) -> Vec<u8> {
        let mut bits = Bits::default();
        self.picture(&mut bits, temporal_reference, 2);
        let width = self.format.mb_width();
        for row in 0..self.format.mb_height() {
            bits.start(row as u8 + 1);
            bits.put(5, QUANT);
            bits.put(1, 0);
            for increment in [1, width - 1] {
                put_address(&mut bits, increment);
                // macroblock_type: motion compensated, not coded (table B.3); motion codes 0 and 0 (table B.10)
                bits.put(3, 0b001);
                bits.put(2, 0b11);
            }
        }
        bits.done()
    }

    fn sequence(&self, bits: &mut Bits) {
        let f = &self.format;
        let hd = f.hd();
        bits.start(SEQUENCE_HEADER);
        bits.put(12, u32::from(f.width));
        bits.put(12, u32::from(f.height));
        bits.put(4, u32::from(f.aspect));
        bits.put(4, u32::from(f.rate));
        // bit rate in 400 bit/s: what the profile and level allow, which is what the stations send too
        bits.put(18, if hd { 48_500 } else { 37_500 });
        bits.put(1, 1);
        // VBV buffer in 16 kbit: Main Level's 112, High Level's 597
        bits.put(10, if hd { 597 } else { 112 });
        // not constrained, the default matrices
        bits.put(3, 0);
        bits.start(EXTENSION);
        bits.put(4, 1);
        // Main Profile at High Level or at Main Level
        bits.put(8, if hd { 0x44 } else { 0x48 });
        bits.put(1, u32::from(f.progressive));
        // 4:2:0, no size extension, no bit rate extension, marker, no VBV extension, B pictures allowed (as a station's), no rate
        // extension
        bits.put(2, 1);
        bits.put(4, 0);
        bits.put(12, 0);
        bits.put(1, 1);
        bits.put(8, 0);
        bits.put(1, 0);
        bits.put(7, 0);
    }

    fn picture(&self, bits: &mut Bits, temporal_reference: u16, coding_type: u32) {
        bits.start(PICTURE);
        bits.put(10, u32::from(temporal_reference & 0x3FF));
        bits.put(3, coding_type);
        bits.put(16, 0xFFFF);
        if coding_type == 2 {
            // full_pel_forward_vector 0, forward_f_code 7: what 13818-2 asks of MPEG-2
            bits.put(4, 0b0111);
        }
        bits.put(1, 0);
        bits.start(EXTENSION);
        bits.put(4, 8);
        // f_codes: forward 1 and 1 for a P picture (its vectors are all zero), the rest unused
        bits.put(16, if coding_type == 2 { 0x11FF } else { 0xFFFF });
        // 8-bit DC, frame picture, top field first in an interlaced sequence, frame prediction and DCT only, no concealment vectors,
        // linear quantiser scale, table B.14, zigzag, no repeated field, 4:2:0 progressive, a progressive frame, no composite flags
        bits.put(2, 0);
        bits.put(2, 3);
        bits.put(1, u32::from(!self.format.progressive));
        bits.put(1, 1);
        bits.put(1, 0);
        bits.put(1, 0);
        bits.put(1, 0);
        bits.put(1, 0);
        bits.put(1, 0);
        bits.put(1, 1);
        bits.put(1, 1);
        bits.put(1, 0);
    }
}

/// One 8x8 intra block at (`x`, `y`) of a plane `stride` wide: its DC as a difference from the last one of the slice, then its AC
/// coefficients when the block is not flat, then end of block.
fn block(bits: &mut Bits, plane: &[u8], stride: usize, x: usize, y: usize, predict: &mut i32, dc_table: &[(u32, u32); 12]) {
    let at = |i: usize| plane[(y + i / 8) * stride + x + i % 8];
    let first = at(0);
    let flat = (1..64).all(|i| at(i) == first);
    let sum: u32 = (0..64).map(|i| u32::from(at(i))).sum();
    let dc = ((sum + 32) / 64) as i32;
    let diff = dc - *predict;
    *predict = dc;
    let size = 32 - diff.unsigned_abs().leading_zeros();
    let (code, len) = dc_table[size as usize];
    bits.put(len, code);
    if size > 0 {
        let v = if diff > 0 { diff } else { diff + (1 << size) - 1 };
        bits.put(size, v as u32);
    }
    if !flat {
        let mut samples = [0f32; 64];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = f32::from(at(i)) - 128.0;
        }
        let coefficients = dct(&samples);
        let scale = 2.0 * QUANT as f32;
        let mut run = 0usize;
        for &z in &ZIGZAG[1..] {
            let z = usize::from(z);
            let q = (16.0 * coefficients[z] / (f32::from(INTRA_MATRIX[z]) * scale)).round() as i32;
            let level = q.clamp(-2047, 2047);
            if level == 0 {
                run += 1;
                continue;
            }
            let magnitude = level.unsigned_abs() as usize;
            match LEVELS_OF_RUN.get(run).filter(|&&most| magnitude <= usize::from(most)) {
                Some(_) => {
                    let index = LEVELS_OF_RUN[..run].iter().map(|&n| usize::from(n)).sum::<usize>() + magnitude - 1;
                    let (code, len) = AC[index];
                    bits.put(u32::from(len), u32::from(code));
                    bits.put(1, u32::from(level < 0));
                }
                None => {
                    bits.put(ESCAPE.1, ESCAPE.0);
                    bits.put(6, run as u32);
                    bits.put(12, (level as u32) & 0xFFF);
                }
            }
            run = 0;
        }
    }
    bits.put(END_OF_BLOCK.1, END_OF_BLOCK.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SD: Format = Format {
        width: 720,
        height: 480,
        rate: 4,
        aspect: 3,
        progressive: false,
    };

    fn picture_of(format: &Format, planes: &Planes) -> Vec<u8> {
        let e = Encoder::new(*format);
        let slices: Vec<_> = (0..format.mb_height()).map(|r| e.intra_slice(planes, r)).collect();
        e.intra(&slices)
    }

    fn slices_in(es: &[u8]) -> usize {
        es.windows(4).filter(|w| w[..3] == [0, 0, 1] && (1..=0xAF).contains(&w[3])).count()
    }

    #[test]
    fn a_written_sequence_header_reads_back_as_its_format() {
        for f in [
            SD,
            Format::FALLBACK,
            Format { width: 1280, height: 720, rate: 7, aspect: 3, progressive: true },
        ] {
            let es = picture_of(&f, &Planes::new(&f, [16, 128, 128]));
            assert_eq!(Format::of_sequence(&es), Some(f));
            assert_eq!(slices_in(&es), f.mb_height(), "a slice a macroblock row");
        }
    }

    #[test]
    fn a_station_sequence_header_with_its_extension_is_read() {
        // 2.1 on RF 32: 1280x720, 16:9, 59.94, progressive (captured 2026-11-05)
        let es = [
            0, 0, 1, 0xB3, 0x50, 0x02, 0xD0, 0x37, 0xFF, 0xFF, 0xE0, 0x18, 0, 0, 1, 0xB5, 0x14, 0x8A, 0, 1, 0, 0,
        ];
        assert_eq!(
            Format::of_sequence(&es),
            Some(Format { width: 1280, height: 720, rate: 7, aspect: 3, progressive: true })
        );
        assert_eq!(Format::of_sequence(&es[..10]), None, "cut short");
    }

    #[test]
    fn a_flat_picture_costs_its_dcs_and_a_repeat_a_few_bytes_a_row() {
        let flat = picture_of(&SD, &Planes::new(&SD, [40, 130, 120]));
        let mut marked = Planes::new(&SD, [40, 130, 120]);
        for y in 200..240 {
            for x in 100..300 {
                marked.y[y * marked.width + x] = if (x / 3 + y / 3) % 2 == 0 { 235 } else { 16 };
            }
        }
        let edged = picture_of(&SD, &marked);
        // 1350 macroblocks of DCs alone, then the busy blocks' coefficients on top
        assert!(flat.len() < 6_000, "{}", flat.len());
        assert!(edged.len() > flat.len() + 2_000, "{} {}", edged.len(), flat.len());
        let repeat = Encoder::new(SD).repeat(3);
        assert_eq!(slices_in(&repeat), SD.mb_height());
        assert!(repeat.len() < 400, "{}", repeat.len());
        assert_eq!(&repeat[..4], &[0, 0, 1, 0]);
        assert_eq!(repeat[5] >> 3 & 7, 2, "a P picture");
    }

    #[test]
    fn the_transform_of_a_flat_block_is_its_dc_alone() {
        let c = dct(&[10.0; 64]);
        assert!((c[0] - 80.0).abs() < 1e-3);
        assert!(c[1..].iter().all(|v| v.abs() < 1e-3));
        let mut edge = [0f32; 64];
        for (i, s) in edge.iter_mut().enumerate() {
            *s = if i % 8 < 4 { 50.0 } else { -50.0 };
        }
        let c = dct(&edge);
        assert!(c[1].abs() > 100.0 && c[8].abs() < 1e-3, "a vertical edge is horizontal frequencies");
    }

    #[test]
    fn frame_times_follow_the_rate() {
        assert_eq!(SD.ticks(1), 3003);
        let p60 = Format { rate: 7, ..SD };
        assert_eq!((p60.ticks(1), p60.ticks(2)), (1501, 3003));
        assert_eq!(Format { width: 1920, height: 1080, ..SD }.mb_height(), 68);
    }
}
