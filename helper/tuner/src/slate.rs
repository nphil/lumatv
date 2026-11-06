//! A slate: the picture the tuner shows in the stream while the channel's own is not coming, saying why (tuning, a weak signal, no
//! signal, the TV using its tuner). It is drawn in the host's theme ([`Look`]: its colours and faces) at the channel's own size and shape,
//! and sent as MPEG-2 ([`crate::mpeg2`]): an intra picture twice a second, and in between pictures that repeat it in a few hundred bytes.
//! Only the macroblock rows that changed since the last intra picture are coded again (the bar, a clock).

use crate::mpeg2::{Encoder, Format, Planes};
use crate::stream::Signal;
use fontdue::{Font, FontSettings};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// The screen a slate is laid out on: 1080 lines, as wide as the picture's shape makes them.
const LINES: f32 = 1080.0;

/// A face at a size (in pixels of a 1080-line picture).
#[derive(Clone)]
pub struct Type {
    pub face: Arc<Font>,
    pub size: f32,
}

impl Type {
    /// A face from a TrueType or OpenType file.
    pub fn new(file: &[u8], size: f32) -> Result<Self, String> {
        let face = Font::from_bytes(file, FontSettings::default()).map_err(str::to_owned)?;
        Ok(Self {
            face: Arc::new(face),
            size,
        })
    }
}

/// How a slate looks: what the host's theme says, in sRGB, and its faces for the headline, the line under it and the small print.
#[derive(Clone)]
pub struct Look {
    pub canvas: [u8; 3],
    pub ink: [u8; 3],
    pub quiet: [u8; 3],
    /// The bar's empty part.
    pub track: [u8; 3],
    pub accent: [u8; 3],
    pub warning: [u8; 3],
    pub danger: [u8; 3],
    pub headline: Type,
    pub line: Type,
    pub small: Type,
}

impl fmt::Debug for Look {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Look")
            .field("canvas", &self.canvas)
            .field("ink", &self.ink)
            .field("accent", &self.accent)
            .field("sizes", &[self.headline.size, self.line.size, self.small.size])
            .finish_non_exhaustive()
    }
}

/// What a slate says.
#[derive(Clone, Debug, PartialEq)]
pub enum Says {
    /// The channel is being tuned: how far along, 0 to 1.
    Tuning(f32),
    /// The picture is not coming in: the signal as the TV reads it, and how long it has been waited for.
    Weak {
        signal: Option<Signal>,
        waited: Duration,
    },
    /// The antenna does not bring the channel in; the stream ends.
    NoSignal,
    /// The TV's own Live TV took the tuner; the stream ends.
    TvInUse,
    /// The program asks not to be passed on; the stream ends.
    Protected,
    /// The tuner said no; the stream ends.
    Failed,
}

/// What a slate draws: a headline, a line under it, maybe a bar (how full, in what colour) and the small print under the bar.
struct Words {
    headline: String,
    line: String,
    bar: Option<(f32, [u8; 3])>,
    small: Option<String>,
}

impl Says {
    fn words(&self, guide: &str, name: &str, look: &Look) -> Words {
        let plain = |headline: String, line: String| Words {
            headline,
            line,
            bar: None,
            small: None,
        };
        match self {
            Self::Tuning(progress) => Words {
                headline: format!("Tuning {guide} {name}\u{2026}"),
                line: "Getting the picture from the antenna".to_owned(),
                bar: Some((progress.clamp(0.0, 1.0), look.accent)),
                small: None,
            },
            Self::Weak { signal, waited } => {
                let secs = waited.as_secs();
                Words {
                    headline: format!("Weak signal on {guide}"),
                    line: format!("Waiting for the antenna\u{2026} {}:{:02}", secs / 60, secs % 60),
                    bar: Some((signal.map_or(0.0, |s| s.strength()), look.warning)),
                    small: Some(signal.map_or_else(|| "The TV does not say how strong the signal is".to_owned(), |s| s.words())),
                }
            }
            Self::NoSignal => plain(
                format!("No signal on {guide}"),
                format!("{guide} {name} is not coming in on the antenna right now"),
            ),
            Self::TvInUse => plain(
                "The TV is using its tuner".to_owned(),
                format!("Live TV on the TV comes first, so {guide} stopped"),
            ),
            Self::Protected => plain(
                format!("{guide} can't be shared"),
                "The broadcaster marks this program as protected".to_owned(),
            ),
            Self::Failed => plain(format!("Couldn't tune {guide}"), "The TV's tuner did not respond".to_owned()),
        }
    }
}

/// sRGB to studio-range Y'CbCr: BT.709 for a high-definition picture, BT.601 otherwise.
fn yuv(rgb: [u8; 3], hd: bool) -> [u8; 3] {
    let (kr, kb) = if hd { (0.2126, 0.0722) } else { (0.299, 0.114) };
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    [16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr].map(|v| v.round().clamp(1.0, 254.0) as u8)
}

/// A coverage mask over a rectangle of the luma plane, blended in one colour.
struct Mask {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Mask {
    fn new(planes: &Planes, x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        // even edges, so each chroma sample's four luma samples are all in or all out
        let x = (x0.floor().max(0.0) as usize) & !1;
        let y = (y0.floor().max(0.0) as usize) & !1;
        let xe = ((x1.ceil().max(0.0) as usize + 1) & !1).min(planes.width);
        let ye = ((y1.ceil().max(0.0) as usize + 1) & !1).min(planes.height);
        let (w, h) = (xe.saturating_sub(x), ye.saturating_sub(y));
        Self {
            x,
            y,
            w,
            h,
            a: vec![0.0; w * h],
        }
    }

    fn add(&mut self, x: isize, y: isize, a: f32) {
        let (dx, dy) = (x - self.x as isize, y - self.y as isize);
        if dx >= 0 && dy >= 0 && (dx as usize) < self.w && (dy as usize) < self.h {
            let v = &mut self.a[dy as usize * self.w + dx as usize];
            *v = (*v + a).min(1.0);
        }
    }

    fn blend(&self, planes: &mut Planes, colour: [u8; 3]) {
        let mix = |old: u8, new: u8, a: f32| (f32::from(old) + (f32::from(new) - f32::from(old)) * a).round() as u8;
        for j in 0..self.h {
            for i in 0..self.w {
                let a = self.a[j * self.w + i];
                if a > 0.0 {
                    let at = (self.y + j) * planes.width + self.x + i;
                    planes.y[at] = mix(planes.y[at], colour[0], a);
                }
            }
        }
        let cw = planes.width / 2;
        for j in (0..self.h).step_by(2) {
            for i in (0..self.w).step_by(2) {
                let a = (self.a[j * self.w + i]
                    + self.a[j * self.w + i + 1]
                    + self.a[(j + 1) * self.w + i]
                    + self.a[(j + 1) * self.w + i + 1])
                    / 4.0;
                if a > 0.0 {
                    let at = (self.y + j) / 2 * cw + (self.x + i) / 2;
                    planes.cb[at] = mix(planes.cb[at], colour[1], a);
                    planes.cr[at] = mix(planes.cr[at], colour[2], a);
                }
            }
        }
    }
}

/// Lays a picture of `format` out on the 1080-line screen: coded pixels per laid-out pixel, across and down.
struct Scale {
    x: f32,
    y: f32,
    /// The laid-out screen's width.
    width: f32,
}

impl Scale {
    fn of(format: &Format) -> Self {
        let width = LINES * format.display_width() / f32::from(format.height);
        Self {
            x: f32::from(format.width) / width,
            y: f32::from(format.height) / LINES,
            width,
        }
    }
}

/// Draws one line of `text` in `ty`, centred on the screen with its baseline at `baseline` (laid-out pixels), cut short with an ellipsis
/// when it is wider than `widest`.
fn text(planes: &mut Planes, scale: &Scale, ty: &Type, text: &str, baseline: f32, widest: f32, colour: [u8; 3]) {
    let px = ty.size * scale.y;
    // glyphs are rasterised at the picture's line scale and squeezed or stretched across to its pixel shape
    let squeeze = scale.x / scale.y;
    let advance = |s: &str| -> f32 {
        let mut w = 0.0;
        let mut last = None;
        for c in s.chars() {
            if let Some(l) = last {
                w += ty.face.horizontal_kern(l, c, px).unwrap_or(0.0);
            }
            w += ty.face.metrics(c, px).advance_width;
            last = Some(c);
        }
        w
    };
    let limit = widest * scale.y;
    let mut shown = text.to_owned();
    if advance(&shown) > limit {
        while !shown.is_empty() && advance(&format!("{shown}\u{2026}")) > limit {
            shown.pop();
        }
        shown = format!("{}\u{2026}", shown.trim_end());
    }
    let width = advance(&shown) * squeeze;
    let left = scale.width / 2.0 * scale.x - width / 2.0;
    let base = baseline * scale.y;
    let lines = ty.face.horizontal_line_metrics(px);
    let (ascent, descent) = lines.map_or((px, px / 4.0), |l| (l.ascent, -l.descent));
    let mut mask = Mask::new(planes, left - 2.0, base - ascent - 2.0, left + width + 2.0, base + descent + 2.0);
    let mut pen = 0.0f32;
    let mut last = None;
    for c in shown.chars() {
        if let Some(l) = last {
            pen += ty.face.horizontal_kern(l, c, px).unwrap_or(0.0);
        }
        let (m, bitmap) = ty.face.rasterize(c, px);
        let x0 = left + (pen + m.xmin as f32) * squeeze;
        let top = (base - (m.ymin as f32 + m.height as f32)).round() as isize;
        let span = (m.width as f32 * squeeze).ceil() as isize + 1;
        for gy in 0..m.height {
            for dx in 0..=span {
                let x = x0.floor() as isize + dx;
                // the source column this coded pixel's centre falls on
                let u = (x as f32 + 0.5 - x0) / squeeze - 0.5;
                if u <= -1.0 || u >= m.width as f32 {
                    continue;
                }
                let i = u.floor();
                let f = u - i;
                let at = |c: f32| -> f32 {
                    if c < 0.0 || c >= m.width as f32 { 0.0 } else { f32::from(bitmap[gy * m.width + c as usize]) / 255.0 }
                };
                let a = at(i) * (1.0 - f) + at(i + 1.0) * f;
                if a > 0.0 {
                    mask.add(x, top + gy as isize, a);
                }
            }
        }
        pen += m.advance_width;
        last = Some(c);
    }
    mask.blend(planes, colour);
}

/// A bar `wide` laid-out pixels across and `tall` down, centred, its top at `top`, rounded at its ends, filled `full` of the way (at
/// least as far as it is tall, so a filled bar is a pill).
fn bar(planes: &mut Planes, scale: &Scale, top: f32, wide: f32, tall: f32, full: f32, colour: [u8; 3]) {
    let left = (scale.width - wide) / 2.0;
    let right = left + (wide * full).max(tall);
    let r = tall / 2.0;
    let mut mask = Mask::new(planes, left * scale.x - 2.0, top * scale.y - 2.0, right * scale.x + 2.0, (top + tall) * scale.y + 2.0);
    for j in 0..mask.h {
        for i in 0..mask.w {
            let (x, y) = ((mask.x + i) as f32 + 0.5, (mask.y + j) as f32 + 0.5);
            let (lx, ly) = (x / scale.x, y / scale.y);
            // the distance from a capsule's spine, less its radius, in coded pixels down
            let cx = lx.clamp(left + r, right - r);
            let d = ((lx - cx).powi(2) + (ly - top - r).powi(2)).sqrt() - r;
            let a = (0.5 - d * scale.y).clamp(0.0, 1.0);
            mask.a[j * mask.w + i] = a;
        }
    }
    mask.blend(planes, colour);
}

/// One channel's slate as it goes out: what it says now, drawn and coded, and where it is in its group of pictures.
pub struct Slate {
    look: Look,
    guide: String,
    name: String,
    encoder: Encoder,
    hd: bool,
    says: Says,
    /// What the coded picture shows, and its rows.
    drawn: Option<(Says, Planes, Vec<Vec<u8>>)>,
    /// Pictures since the last intra picture, and how many a group has.
    since: u16,
    group: u16,
}

impl Slate {
    pub fn new(look: Look, guide: &str, name: &str, format: Format) -> Self {
        Self {
            look,
            guide: guide.to_owned(),
            name: name.to_owned(),
            hd: format.hd(),
            group: (format.fps() / 2.0).round().max(1.0) as u16,
            encoder: Encoder::new(format),
            says: Says::Tuning(0.0),
            drawn: None,
            since: 0,
        }
    }

    pub fn format(&self) -> &Format {
        self.encoder.format()
    }

    /// From the next intra picture on, it says `says`.
    pub fn say(&mut self, says: Says) {
        self.says = says;
    }

    pub fn says(&self) -> &Says {
        &self.says
    }

    /// The next picture: an intra one at the start of each group (drawn again when what it says has changed), a repeat in between.
    pub fn next_picture(&mut self) -> Vec<u8> {
        if self.drawn.is_some() && self.since % self.group != 0 {
            self.since += 1;
            return self.encoder.repeat(self.since - 1);
        }
        self.since = 1;
        if self.drawn.as_ref().is_none_or(|(said, _, _)| *said != self.says) {
            let planes = self.draw();
            let rows = self.encoder.format().mb_height();
            let slices = (0..rows)
                .map(|r| match &self.drawn {
                    // a macroblock row whose samples did not change keeps its coded slice
                    Some((_, old, slices)) if same_row(old, &planes, r) => slices[r].clone(),
                    _ => self.encoder.intra_slice(&planes, r),
                })
                .collect();
            self.drawn = Some((self.says.clone(), planes, slices));
        }
        let (_, _, slices) = self.drawn.as_ref().expect("drawn above");
        self.encoder.intra(slices)
    }

    fn draw(&self) -> Planes {
        let format = *self.encoder.format();
        let look = &self.look;
        let scale = Scale::of(&format);
        let mut planes = Planes::new(&format, yuv(look.canvas, self.hd));
        let words = self.says.words(&self.guide, &self.name, look);
        let widest = scale.width - 160.0;
        text(&mut planes, &scale, &look.headline, &words.headline, 500.0, widest, yuv(look.ink, self.hd));
        text(&mut planes, &scale, &look.line, &words.line, 590.0, widest, yuv(look.quiet, self.hd));
        if let Some((full, colour)) = words.bar {
            bar(&mut planes, &scale, 650.0, 640.0, 12.0, 1.0, yuv(look.track, self.hd));
            if full > 0.0 {
                bar(&mut planes, &scale, 650.0, 640.0, 12.0, full, yuv(colour, self.hd));
            }
        }
        if let Some(small) = &words.small {
            text(&mut planes, &scale, &look.small, small, 720.0, widest, yuv(look.quiet, self.hd));
        }
        planes
    }
}

/// Whether macroblock row `row` of two pictures has the same samples.
fn same_row(a: &Planes, b: &Planes, row: usize) -> bool {
    let (w, cw) = (a.width, a.width / 2);
    let luma = row * 16 * w..(row + 1) * 16 * w;
    let chroma = row * 8 * cw..(row + 1) * 8 * cw;
    a.y[luma.clone()] == b.y[luma] && a.cb[chroma.clone()] == b.cb[chroma.clone()] && a.cr[chroma.clone()] == b.cr[chroma]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The theme the TV ships (Rose Pine, Manrope), as the plugin hands it over.
    pub(crate) fn look() -> Look {
        let medium = include_bytes!("../../../payload/fonts/Manrope-Medium.ttf");
        let semibold = include_bytes!("../../../payload/fonts/Manrope-SemiBold.ttf");
        Look {
            canvas: [0x19, 0x17, 0x24],
            ink: [0xF2, 0xEF, 0xF6],
            quiet: [0xA9, 0xA6, 0xB0],
            track: [0x26, 0x24, 0x30],
            accent: [0xA7, 0x9B, 0xF5],
            warning: [0xF6, 0xC1, 0x77],
            danger: [0xEB, 0x6F, 0x92],
            headline: Type::new(medium, 92.0).unwrap(),
            line: Type::new(semibold, 40.0).unwrap(),
            small: Type::new(medium, 28.0).unwrap(),
        }
    }

    const SD: Format = Format {
        width: 720,
        height: 480,
        rate: 4,
        aspect: 3,
        progressive: false,
    };

    #[test]
    fn a_slate_is_an_intra_picture_then_repeats_twice_a_second() {
        let mut s = Slate::new(look(), "16.9", "WXYZ", SD);
        let first = s.next_picture();
        assert_eq!(Format::of_sequence(&first), Some(SD));
        let repeats: Vec<_> = (0..14).map(|_| s.next_picture()).collect();
        assert!(repeats.iter().all(|p| p.len() < 400 && Format::of_sequence(p).is_none()));
        assert!(Format::of_sequence(&s.next_picture()).is_some(), "the sixteenth starts the next group");
    }

    #[test]
    fn the_words_are_drawn_in_the_ink_on_the_canvas_and_only_changed_rows_are_coded_again() {
        let mut s = Slate::new(look(), "16.9", "WXYZ", SD);
        s.next_picture();
        let (_, planes, slices) = s.drawn.clone().unwrap();
        let canvas = yuv(look().canvas, false)[0];
        let ink = yuv(look().ink, false)[0];
        // the headline's band holds ink, the top of the screen only canvas
        let band = |from: usize, to: usize| planes.y[from * planes.width..to * planes.width].iter().copied().max().unwrap();
        assert!(band(170, 225) > ink - 20, "the headline");
        assert_eq!(band(0, 100), canvas);
        s.say(Says::Tuning(0.5));
        for _ in 0..15 {
            s.next_picture();
        }
        let (_, _, again) = s.drawn.clone().unwrap();
        let changed: Vec<usize> = (0..slices.len()).filter(|&r| slices[r] != again[r]).collect();
        // the bar sits at 650 to 662 of 1080 lines, 289 to 294 of 480: macroblock row 18
        assert_eq!(changed, [18], "only the bar's row");
    }

    #[test]
    fn the_signal_and_the_wait_are_said() {
        let w = Says::Weak {
            signal: Some(Signal { locked: true, mse: Some(325), snr: Some(18.2), errors: Some(0), ber: Some(0) }),
            waited: Duration::from_secs(75),
        }
        .words("16.9", "WXYZ", &look());
        assert_eq!(w.headline, "Weak signal on 16.9");
        assert_eq!(w.line, "Waiting for the antenna\u{2026} 1:15");
        assert_eq!(w.small.as_deref(), Some("Signal 18 dB: weak"));
        assert!(w.bar.is_some_and(|(full, _)| full > 0.0 && full < 0.3));
        let t = Says::Tuning(0.4).words("2.1", "WSB-HD", &look());
        assert_eq!(t.headline, "Tuning 2.1 WSB-HD\u{2026}");
    }

    #[test]
    fn colours_follow_the_picture_s_matrix() {
        assert_eq!(yuv([255, 255, 255], true), [235, 128, 128]);
        assert_eq!(yuv([0, 0, 0], false), [16, 128, 128]);
        assert_ne!(yuv([255, 0, 0], true), yuv([255, 0, 0], false));
    }
}
