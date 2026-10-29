//! The digits of a clock: `7:42`, `07:42` or `07:42:09` on one layer, in cells of one width each so that a changing digit never
//! moves its neighbours (tabular numerals, whatever the font's own advances are). Only the cells that changed are drawn again; a
//! minute tick is one small upload, not the whole face.

use super::Damage;
use super::label::Align;
use super::text::{aligned, line_height};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::scene::ImageLayer;
use crate::text::{Canvas, Type};
use compositor_protocol::{Props, Rect, Rgba};
use std::io;

/// A time of day as the digits show it. Hours are 0 to 23, minutes and seconds 0 to 59; what is outside is shown as its last
/// two digits would be, never a longer string, so a layout made for the longest reading always holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeDigits {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl TimeDigits {
    pub const fn new(hour: u8, minute: u8, second: u8) -> Self {
        Self {
            hour,
            minute,
            second,
        }
    }
}

/// How the digits are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockFormat {
    /// `07:42:09` rather than `07:42`.
    pub seconds: bool,
    /// `07:42` rather than `7:42`.
    pub leading_zero: bool,
}

impl ClockFormat {
    pub const QUIET: ClockFormat = ClockFormat {
        seconds: false,
        leading_zero: false,
    };

    /// The reading, as characters.
    pub fn text(self, time: TimeDigits) -> String {
        let (h, m, s) = (time.hour % 100, time.minute % 100, time.second % 100);
        let mut out = if self.leading_zero {
            format!("{h:02}:{m:02}")
        } else {
            format!("{h}:{m:02}")
        };
        if self.seconds {
            out.push_str(&format!(":{s:02}"));
        }
        out
    }
}

/// How a face sets its cells when they are not the font's own (the night clock's, which is set by its mock-up): the width of the cell
/// of a digit and of the colon, where the baseline is in the layer, and how far the colon is raised from it (the font's colon sits at
/// its x-height, and a clock reads best with it at the middle of the digits). The digits are the font's tabular figures
/// ([`crate::text::TABULAR_EM`] wide, as a browser sets `tabular-nums`), and each is drawn as a browser draws text in a box: centred
/// in its cell when it fits, from the cell's left edge when it is wider than the cell (the pitch is then a tracking, and the digit
/// overflows its cell on the right, where its own side bearing is).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DigitCells {
    pub digit: f32,
    pub colon: f32,
    /// The baseline's distance from the layer's top.
    pub baseline: f32,
    /// How far above the baseline the colon is drawn.
    pub colon_lift: f32,
}

/// The digits on one layer. The layer is the frame's size and never changes; the reading is placed in it by the alignment.
pub struct ClockDigits {
    layer: ImageLayer,
    frame: Frame,
    ty: Type,
    colour: Rgba,
    align: Align,
    format: ClockFormat,
    /// The width of every digit's cell and of the colon's.
    digit_w: f32,
    colon_w: f32,
    /// How the cells were set when the font's own were not used.
    set: Option<DigitCells>,
    time: TimeDigits,
    /// What is drawn: each character with the left edge of its cell, in layer pixels.
    drawn: Vec<(char, f32)>,
}

impl ClockDigits {
    /// The cell widths of `ty`: the widest digit, and the colon.
    fn cells(ui: &mut Ui, ty: Type) -> (f32, f32) {
        let digit = ('0'..='9')
            .map(|c| ui.fonts.measure(&c.to_string(), ty).width)
            .fold(0.0f32, f32::max)
            .ceil();
        let colon = ui.fonts.measure(":", ty).width.ceil();
        (digit, colon)
    }

    /// The width the longest reading of `format` needs at `ty` (two digit hours).
    pub fn required_width(ui: &mut Ui, ty: Type, format: ClockFormat) -> f32 {
        let (digit, colon) = Self::cells(ui, ty);
        if format.seconds {
            6.0 * digit + 2.0 * colon
        } else {
            4.0 * digit + colon
        }
    }

    /// The height of the layer for `ty`: one line.
    pub fn required_height(ty: Type) -> f32 {
        line_height(ty)
    }

    /// Digits in `frame` showing `time`. The frame must hold the longest reading ([`ClockDigits::required_width`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        ty: Type,
        colour: Rgba,
        align: Align,
        format: ClockFormat,
        time: TimeDigits,
    ) -> io::Result<Self> {
        let (digit_w, colon_w) = Self::cells(ui, ty);
        let need = Self::required_width(ui, ty, format);
        if frame.w + 0.5 < need || frame.h + 0.5 < line_height(ty) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "clock digits of {} px need {need} x {} units, not {} x {}",
                    ty.px,
                    line_height(ty),
                    frame.w,
                    frame.h
                ),
            ));
        }
        Self::made(
            ui,
            frame,
            z,
            ty,
            colour,
            align,
            format,
            time,
            (digit_w, colon_w),
            None,
        )
    }

    /// Digits in `frame` showing `time`, their cells set as `cells` says and not as the font does. The frame must hold the longest
    /// reading of `format` in those cells and the baseline must be inside it.
    #[allow(clippy::too_many_arguments)]
    pub fn with_cells(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        ty: Type,
        colour: Rgba,
        align: Align,
        format: ClockFormat,
        time: TimeDigits,
        cells: DigitCells,
    ) -> io::Result<Self> {
        let need = if format.seconds {
            6.0 * cells.digit + 2.0 * cells.colon
        } else {
            4.0 * cells.digit + cells.colon
        };
        if frame.w + 0.5 < need
            || cells.baseline > frame.h
            || cells.digit <= 0.0
            || cells.colon <= 0.0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "clock digits in cells of {} and {} with a baseline at {} need {need} units across and the baseline inside {} x {}",
                    cells.digit, cells.colon, cells.baseline, frame.w, frame.h
                ),
            ));
        }
        // All ten digits and the colon are rastered now, so that a tick never pays for a glyph.
        ui.fonts.warm_tabular("0123456789:", ty);
        Self::made(
            ui,
            frame,
            z,
            ty,
            colour,
            align,
            format,
            time,
            (cells.digit, cells.colon),
            Some(cells),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn made(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        ty: Type,
        colour: Rgba,
        align: Align,
        format: ClockFormat,
        time: TimeDigits,
        (digit_w, colon_w): (f32, f32),
        set: Option<DigitCells>,
    ) -> io::Result<Self> {
        let layer = ui.scene.image(
            frame.w.ceil() as u32,
            frame.h.ceil() as u32,
            z,
            Props::at(frame.x, frame.y),
        )?;
        let mut digits = Self {
            layer,
            frame,
            ty,
            colour,
            align,
            format,
            digit_w,
            colon_w,
            set,
            time,
            drawn: Vec::new(),
        };
        digits.draw(ui, true);
        Ok(digits)
    }

    pub fn id(&self) -> compositor_protocol::LayerId {
        self.layer.id()
    }

    /// The digits' compositor layer, to hang it under a native group.
    pub fn layer(&self) -> &crate::scene::Layer {
        &self.layer
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn time(&self) -> TimeDigits {
        self.time
    }

    /// What the digits read now.
    pub fn text(&self) -> String {
        self.drawn.iter().map(|(c, _)| *c).collect()
    }

    /// Shows `time`. Only the cells whose character changed are drawn again; when the reading changes length (`9:59` to `10:00`)
    /// the whole reading is placed again.
    pub fn set_time(&mut self, ui: &mut Ui, time: TimeDigits) -> Damage {
        if time == self.time {
            return Damage::None;
        }
        self.time = time;
        self.draw(ui, false)
    }

    pub fn set_colour(&mut self, ui: &mut Ui, colour: Rgba) -> Damage {
        if colour == self.colour {
            return Damage::None;
        }
        self.colour = colour;
        self.draw(ui, true)
    }

    /// Writes the hour with its leading zero (`07:42`) or without (`7:42`) from now on, as the person moves between the 24-hour and the
    /// 12-hour clock: the whole reading is drawn again. Neither is longer than the frame was made for (four digits and a colon).
    pub fn set_leading_zero(&mut self, ui: &mut Ui, leading_zero: bool) -> Damage {
        if self.format.leading_zero == leading_zero {
            return Damage::None;
        }
        self.format.leading_zero = leading_zero;
        self.draw(ui, true)
    }

    /// Moves the layer at once, cancelling any animation on it.
    pub fn set_props(&mut self, props: Props) {
        self.layer.set(props);
    }

    /// Animates the layer's position, opacity and scale.
    pub fn animate_props(&mut self, to: Props, duration_ms: u32, ease: compositor_protocol::Ease) {
        self.layer.animate(to, duration_ms, ease);
    }

    fn cell_w(&self, c: char) -> f32 {
        if c == ':' { self.colon_w } else { self.digit_w }
    }

    /// Draws the reading: all of it when `all`, or when its layout changed; else only the cells that differ.
    fn draw(&mut self, ui: &mut Ui, all: bool) -> Damage {
        let text = self.format.text(self.time);
        let width: f32 = text.chars().map(|c| self.cell_w(c)).sum();
        let mut x = aligned(0.0, self.frame.w, width, self.align).round();
        let mut next: Vec<(char, f32)> = Vec::with_capacity(text.len());
        for c in text.chars() {
            next.push((c, x));
            x += self.cell_w(c);
        }
        let same_layout = !all
            && next.len() == self.drawn.len()
            && next.iter().zip(&self.drawn).all(|(a, b)| a.1 == b.1);
        let baseline = match self.set {
            Some(cells) => cells.baseline,
            None => {
                let extent = ui.fonts.measure("0", self.ty);
                ((self.frame.h - extent.height()) / 2.0 + extent.ascent).round()
            }
        };
        let (lw, lh) = (self.layer.width() as usize, self.layer.height() as usize);
        if !same_layout {
            self.layer.clear();
        }
        let mut changed = false;
        for (i, &(c, left)) in next.iter().enumerate() {
            if same_layout && self.drawn[i].0 == c {
                continue;
            }
            let cell = self.cell_w(c);
            if same_layout {
                clear_cell(self.layer.pixels_mut(), lw, lh, left, cell);
            }
            // A set cell is drawn with the font's tabular figures; the font's own cell with the plain ones.
            let tabular = self.set.is_some();
            let s = c.to_string();
            let advance = if tabular {
                ui.fonts.measure_tabular(&s, self.ty).width
            } else {
                ui.fonts.measure(&s, self.ty).width
            };
            // Centred in its cell, or from the cell's left edge when it is wider than the cell: the font's own cell is as wide as
            // its widest digit, a set one may be narrower (a tracking), and a browser starts text that does not fit its box.
            let at = left + ((cell - advance) / 2.0).max(0.0);
            let lift = match self.set {
                Some(cells) if c == ':' => cells.colon_lift,
                _ => 0.0,
            };
            let mut canvas = Canvas {
                pixels: self.layer.pixels_mut(),
                width: lw,
            };
            if tabular {
                ui.fonts
                    .draw_tabular(&mut canvas, &s, self.ty, (at, baseline - lift), self.colour);
            } else {
                ui.fonts
                    .draw(&mut canvas, &s, self.ty, (at, baseline - lift), self.colour);
            }
            if same_layout {
                self.layer.damage(Some(Rect {
                    x: left.floor(),
                    y: 0.0,
                    w: cell.ceil(),
                    h: lh as f32,
                }));
            }
            changed = true;
        }
        if !same_layout {
            self.layer.damage(None);
            changed = true;
        }
        self.drawn = next;
        if changed {
            Damage::Content
        } else {
            Damage::None
        }
    }

    /// The restyled look: the digits are drawn again in the new ink.
    pub fn restyle(&mut self, ui: &mut Ui, colour: Rgba) {
        self.colour = colour;
        self.draw(ui, true);
    }
}

/// Clears the cell at `left` (`cell` wide) of a layer `w` x `h` pixels.
fn clear_cell(pixels: &mut [u8], w: usize, h: usize, left: f32, cell: f32) {
    let from = (left.floor().max(0.0) as usize).min(w);
    let to = ((left + cell).ceil().max(0.0) as usize).min(w);
    for y in 0..h {
        pixels[(y * w + from) * 4..(y * w + to) * 4].fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::harness;

    fn digits(
        h: &mut crate::testing::Harness,
        format: ClockFormat,
        time: TimeDigits,
    ) -> ClockDigits {
        let ty = Type {
            px: 72,
            weight: crate::text::Weight::Medium,
        };
        let w = ClockDigits::required_width(&mut h.ui, ty, format);
        let frame = Frame::new(10.0, 20.0, w + 8.0, ClockDigits::required_height(ty));
        ClockDigits::new(
            &mut h.ui,
            frame,
            5,
            ty,
            Rgba(255, 255, 255, 255),
            Align::Centre,
            format,
            time,
        )
        .unwrap()
    }

    #[test]
    fn the_reading_is_written_the_way_the_format_says() {
        let t = TimeDigits::new(7, 42, 9);
        assert_eq!(ClockFormat::QUIET.text(t), "7:42");
        assert_eq!(
            ClockFormat {
                seconds: false,
                leading_zero: true
            }
            .text(t),
            "07:42"
        );
        assert_eq!(
            ClockFormat {
                seconds: true,
                leading_zero: true
            }
            .text(t),
            "07:42:09"
        );
        // Out of range is its last two digits, never a longer string.
        assert_eq!(
            ClockFormat {
                seconds: true,
                leading_zero: true
            }
            .text(TimeDigits::new(124, 65, 200)),
            "24:65:00"
        );
    }

    #[test]
    fn a_digit_that_changes_is_drawn_alone_and_its_neighbours_stay_where_they_are() {
        let mut h = harness();
        let mut d = digits(
            &mut h,
            ClockFormat {
                seconds: false,
                leading_zero: true,
            },
            TimeDigits::new(7, 42, 0),
        );
        h.commit();
        let before = h.ui.fonts.cached_glyphs();
        let pixels_before = h.pixels(d.id());
        assert_eq!(
            d.set_time(&mut h.ui, TimeDigits::new(7, 43, 0)),
            Damage::Content
        );
        let sent = h.commit();
        let pixels_after = h.pixels(d.id());
        assert_eq!(d.text(), "07:43");
        // The minutes' last digit changed; the hours and the colon are the same bytes.
        let row = d.layer.width() as usize * 4;
        let first_diff = pixels_before
            .chunks(row)
            .zip(pixels_after.chunks(row))
            .filter_map(|(a, b)| a.iter().zip(b).position(|(x, y)| x != y))
            .min()
            .expect("a pixel changed");
        let (digit, colon) = (d.digit_w, d.colon_w);
        let last_cell_left = d.drawn[4].1;
        assert!(
            first_diff as f32 / 4.0 >= last_cell_left - 1.0,
            "pixels left of the last digit changed: first at {first_diff}, cell at {last_cell_left} ({digit}, {colon})"
        );
        assert!(
            sent.iter()
                .any(|r| matches!(r, compositor_protocol::Request::Damage { .. })),
            "the upload is a damage of the cell, not a layer created again: {sent:?}"
        );
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, compositor_protocol::Request::Create { .. })),
            "no layer is made for a tick"
        );
        assert_eq!(
            h.ui.fonts.cached_glyphs(),
            before,
            "no glyph is rastered for a tick: the digits were warm"
        );
    }

    #[test]
    fn every_digit_takes_one_cell_so_nothing_moves_when_a_one_becomes_an_eight() {
        let mut h = harness();
        let d = digits(
            &mut h,
            ClockFormat {
                seconds: true,
                leading_zero: true,
            },
            TimeDigits::new(11, 11, 11),
        );
        let ones: Vec<f32> = d.drawn.iter().map(|d| d.1).collect();
        let mut d = d;
        d.set_time(&mut h.ui, TimeDigits::new(18, 88, 88));
        let eights: Vec<f32> = d.drawn.iter().map(|d| d.1).collect();
        assert_eq!(ones, eights, "the cells are where they were");
    }

    #[test]
    fn a_reading_that_changes_length_is_placed_again_and_a_frame_too_small_is_an_error() {
        let mut h = harness();
        let mut d = digits(&mut h, ClockFormat::QUIET, TimeDigits::new(9, 59, 0));
        let nine = d.drawn[0].1;
        d.set_time(&mut h.ui, TimeDigits::new(10, 0, 0));
        assert_eq!(d.text(), "10:00");
        assert!(
            d.drawn[0].1 < nine,
            "five cells are centred wider than four"
        );
        let ty = Type {
            px: 72,
            weight: crate::text::Weight::Medium,
        };
        let small = Frame::new(0.0, 0.0, 40.0, 100.0);
        assert!(
            ClockDigits::new(
                &mut h.ui,
                small,
                1,
                ty,
                Rgba(1, 1, 1, 255),
                Align::Left,
                ClockFormat::QUIET,
                TimeDigits::new(1, 2, 3)
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod set_cells_tests {
    use super::*;
    use crate::testing::{Harness, harness};
    use crate::text::Weight;

    const CELLS: DigitCells = DigitCells {
        digit: 124.0,
        colon: 62.0,
        baseline: 184.0,
        colon_lift: 20.0,
    };
    const INK: Rgba = Rgba(227, 59, 53, 255);

    fn type_208() -> Type {
        Type {
            px: 208,
            weight: Weight::Medium,
        }
    }

    fn digits(
        h: &mut Harness,
        format: ClockFormat,
        time: TimeDigits,
        cells: DigitCells,
    ) -> ClockDigits {
        ClockDigits::with_cells(
            &mut h.ui,
            Frame::new(201.0, 132.0, 558.0, 208.0),
            5,
            type_208(),
            INK,
            Align::Centre,
            format,
            time,
            cells,
        )
        .unwrap()
    }

    const LEADING: ClockFormat = ClockFormat {
        seconds: false,
        leading_zero: true,
    };

    /// The rows (first, last + 1) and columns (first, last + 1) of the layer's pixels that have any ink, within columns `from..to`.
    fn ink(
        pixels: &[u8],
        width: usize,
        from: usize,
        to: usize,
    ) -> Option<((usize, usize), (usize, usize))> {
        let (mut rows, mut cols) = ((usize::MAX, 0), (usize::MAX, 0));
        for (i, px) in pixels.chunks_exact(4).enumerate() {
            let (x, y) = (i % width, i / width);
            if px[3] > 0 && (from..to).contains(&x) {
                rows = (rows.0.min(y), rows.1.max(y + 1));
                cols = (cols.0.min(x), cols.1.max(x + 1));
            }
        }
        (rows.0 != usize::MAX).then_some((rows, cols))
    }

    #[test]
    fn set_cells_put_each_digit_in_its_own_cell_and_the_baseline_where_it_was_said() {
        let mut h = harness();
        let d = digits(&mut h, LEADING, TimeDigits::new(11, 11, 0), CELLS);
        // Four cells and a colon fill the frame exactly: the cells are 124 apart, the colon's 62 between the pairs.
        let lefts: Vec<f32> = d.drawn.iter().map(|d| d.1).collect();
        assert_eq!(lefts, [0.0, 124.0, 248.0, 310.0, 434.0]);
        let pixels = h.pixels(d.id());
        // A one is flat at its foot: its ink ends on the baseline, whatever the font's own line is.
        let (rows, _) = ink(&pixels, 558, 0, 124).expect("the first digit has ink");
        assert_eq!(rows.1, 184, "the foot of a 1 is on the baseline");
        assert!(
            (rows.1 - rows.0) as f32 > 0.70 * 208.0 && ((rows.1 - rows.0) as f32) < 0.76 * 208.0,
            "a digit is about 0.72 em tall: {} px",
            rows.1 - rows.0
        );
    }

    #[test]
    fn the_colon_is_raised_by_its_lift_and_nothing_else_of_it_moves() {
        let mut h = harness();
        let flat = digits(
            &mut h,
            LEADING,
            TimeDigits::new(11, 11, 0),
            DigitCells {
                colon_lift: 0.0,
                ..CELLS
            },
        );
        let raised = digits(&mut h, LEADING, TimeDigits::new(11, 11, 0), CELLS);
        let colon = |d: &ClockDigits| {
            let pixels = h.pixels(d.id());
            // The colon's cell: 248 to 310 of the layer.
            ink(&pixels, 558, 248, 310).expect("the colon has ink")
        };
        let (a, b) = (colon(&flat), colon(&raised));
        assert_eq!(a.0.0, b.0.0 + 20, "the top of the colon is 20 rows higher");
        assert_eq!(a.0.1, b.0.1 + 20, "and its foot");
        assert_eq!(a.1, b.1, "in the same columns");
        assert!(b.0.0 >= 24 && b.0.1 <= 184, "inside the digits' height");
    }

    #[test]
    fn a_minute_redraws_one_cell_of_the_layer_and_the_colon_and_the_others_keep_their_pixels() {
        let mut h = harness();
        let mut d = digits(&mut h, LEADING, TimeDigits::new(6, 42, 0), CELLS);
        h.commit();
        let before = h.pixels(d.id());
        assert_eq!(
            d.set_time(&mut h.ui, TimeDigits::new(6, 43, 0)),
            Damage::Content
        );
        let sent = h.commit();
        let after = h.pixels(d.id());
        assert_eq!(d.text(), "06:43");
        // Only the last digit's cell (x 434..558) differs.
        let width = 558 * 4;
        for (row, (a, b)) in before.chunks(width).zip(after.chunks(width)).enumerate() {
            assert_eq!(
                a[..434 * 4],
                b[..434 * 4],
                "row {row} left of the last cell"
            );
        }
        assert_ne!(before, after);
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, compositor_protocol::Request::Create { .. })),
            "no layer is made for a tick"
        );
    }

    #[test]
    fn the_12_hour_reading_is_three_cells_and_the_colon_centred_in_the_same_frame() {
        let mut h = harness();
        let twelve = ClockFormat {
            seconds: false,
            leading_zero: false,
        };
        let mut d = digits(&mut h, twelve, TimeDigits::new(6, 42, 0), CELLS);
        assert_eq!(d.text(), "6:42");
        let lefts: Vec<f32> = d.drawn.iter().map(|d| d.1).collect();
        // 3 x 124 + 62 = 434 centred in 558: the group starts 62 in, which is where the mock-up has it (x 263 on the panel).
        assert_eq!(lefts, [62.0, 186.0, 248.0, 372.0]);
        d.set_time(&mut h.ui, TimeDigits::new(10, 0, 0));
        let lefts: Vec<f32> = d.drawn.iter().map(|d| d.1).collect();
        assert_eq!(
            lefts,
            [0.0, 124.0, 248.0, 310.0, 434.0],
            "ten o'clock is the full width"
        );
    }

    #[test]
    fn cells_that_cannot_hold_the_reading_or_a_baseline_outside_the_frame_are_refused() {
        let mut h = harness();
        let make = |h: &mut Harness, frame: Frame, cells: DigitCells| {
            ClockDigits::with_cells(
                &mut h.ui,
                frame,
                1,
                type_208(),
                INK,
                Align::Centre,
                LEADING,
                TimeDigits::new(1, 2, 3),
                cells,
            )
        };
        let frame = Frame::new(0.0, 0.0, 558.0, 208.0);
        assert!(make(&mut h, frame, CELLS).is_ok());
        assert!(
            make(&mut h, Frame::new(0.0, 0.0, 557.0, 208.0), CELLS).is_err(),
            "a pixel short"
        );
        assert!(
            make(
                &mut h,
                frame,
                DigitCells {
                    baseline: 209.0,
                    ..CELLS
                }
            )
            .is_err(),
            "the baseline below the layer"
        );
        assert!(
            make(
                &mut h,
                frame,
                DigitCells {
                    digit: 0.0,
                    ..CELLS
                }
            )
            .is_err()
        );
    }
}
