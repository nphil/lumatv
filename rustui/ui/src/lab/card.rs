//! The test cards of the lab, rastered on the CPU into one opaque 1920 by 1080 page of pixels (premultiplied RGBA, so the layer is the
//! picture and nothing else): what is drawn is what the display shows, one pixel of the page being one pixel of the UI, which the OSD scaler
//! then magnifies. Letters are the shell's own face (Manrope, through the same rasteriser the glyph text uses), at the sizes the sharpener
//! is judged on (12 to 24 px) whatever the person's text size is, so they are what any text of the shell becomes under the setting.
//!
//! A card is a test picture, not a screen of the shell: it is the same black and white in the light look as in the dark one.

use super::plan::Card;
use compositor_protocol::Rgba;
use ui::text::{Canvas, Fonts, Type, Weight};

pub const W: usize = 1920;
pub const H: usize = 1080;
/// Bytes in a page.
pub const BYTES: usize = W * H * 4;

const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

struct Page<'a> {
    px: &'a mut [u8],
}

impl Page<'_> {
    fn dot(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
            return;
        }
        let i = (y as usize * W + x as usize) * 4;
        self.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        for yy in y.max(0)..(y + h).min(H as i32) {
            for xx in x.max(0)..(x + w).min(W as i32) {
                self.dot(xx, yy, c);
            }
        }
    }

    /// A rectangle with rounded corners of `r` pixels.
    fn round(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: impl Fn(i32, i32) -> [u8; 3]) {
        for yy in y..y + h {
            for xx in x..x + w {
                let (dx, dy) = ((r - (xx - x)).max(xx - (x + w - 1 - r)).max(0), (r - (yy - y)).max(yy - (y + h - 1 - r)).max(0));
                if dx * dx + dy * dy <= r * r {
                    self.dot(xx, yy, c(xx - x, yy - y));
                }
            }
        }
    }

    fn text(&mut self, fonts: &mut Fonts, text: &str, face: (u32, Weight), x: f32, baseline: f32, c: [u8; 3]) -> f32 {
        let mut canvas = Canvas { pixels: &mut *self.px, width: W };
        fonts.draw(&mut canvas, text, Type { px: face.0, weight: face.1 }, (x, baseline), Rgba(c[0], c[1], c[2], 255))
    }

    /// Lines of text one under the other from `y`, each in its size; returns where the next would start.
    fn lines(&mut self, fonts: &mut Fonts, x: f32, y: f32, gap: f32, ink: [u8; 3], rows: &[(u32, &str)]) -> f32 {
        let mut top = y;
        for (px, text) in rows {
            let e = fonts.measure(text, Type { px: *px, weight: Weight::Regular });
            self.text(fonts, text, (*px, Weight::Regular), x, (top + e.ascent).round(), ink);
            top += e.height().ceil() + gap;
        }
        top
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t.clamp(0.0, 1.0)).round() as u8;
    [m(0), m(1), m(2)]
}

/// The colour of a hue (0 to 1) at full saturation and value.
fn hue(h: f32) -> [u8; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let t = h.fract();
    let (up, down) = ((255.0 * t).round() as u8, (255.0 * (1.0 - t)).round() as u8);
    match h.floor() as i32 {
        0 => [255, up, 0],
        1 => [down, 255, 0],
        2 => [0, 255, up],
        3 => [0, down, 255],
        4 => [up, 0, 255],
        _ => [255, 0, down],
    }
}

/// The lines of text a card sets, in the sizes the sharpener is judged on.
const SAMPLE: [(u32, &str); 5] = [
    (12, "12 px  The quick brown fox jumps over the lazy dog 0123456789"),
    (14, "14 px  Sphinx of black quartz, judge my vow. AVWT.,;:"),
    (16, "16 px  Pack my box with five dozen liquor jugs 12345"),
    (20, "20 px  How vexingly quick daft zebras jump"),
    (24, "24 px  Waltz, bad nymph, for quick jigs vex"),
];

/// Paints `card` into `pixels` (a whole page: [`BYTES`] long).
pub fn paint(card: Card, pixels: &mut [u8], fonts: &mut Fonts) {
    assert_eq!(pixels.len(), BYTES, "a card is a whole page");
    let mut page = Page { px: pixels };
    page.rect(0, 0, W as i32, H as i32, BLACK);
    let z = card.zone();
    let (x, y, w, h) = (z.x as i32, z.y as i32, z.w as i32, z.h as i32);
    match card {
        Card::Text => {
            // black on white above, white on black below: a halo shows on one and a thickening on the other
            page.rect(x, y, w, 290, WHITE);
            page.lines(fonts, (x + 16) as f32, (y + 16) as f32, 10.0, BLACK, &SAMPLE);
            page.rect(x, y + 310, w, 290, [18, 18, 18]);
            page.lines(fonts, (x + 16) as f32, (y + 326) as f32, 10.0, WHITE, &SAMPLE);
        }
        Card::Lines => {
            // four tiles of one-pixel structure: rows, columns, a checkerboard and a diagonal
            let (tw, th, gap) = ((w - 20) / 2, (h - 20) / 2, 20);
            let tiles = [(x, y), (x + tw + gap, y), (x, y + th + gap), (x + tw + gap, y + th + gap)];
            for (n, (tx, ty)) in tiles.into_iter().enumerate() {
                for yy in 0..th {
                    for xx in 0..tw {
                        let on = match n {
                            0 => yy % 2 == 0,
                            1 => xx % 2 == 0,
                            2 => (xx + yy) % 2 == 0,
                            _ => (xx + yy) % 4 < 2,
                        };
                        page.dot(tx + xx, ty + yy, if on { WHITE } else { BLACK });
                    }
                }
            }
        }
        Card::Colour => {
            // patches of the colours a picture is made of, a ramp of greys, a sweep of hues, and letters in colour and black on white
            let patches = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0], [255, 255, 255], [128, 128, 128]];
            let pw = (w - 7 * 6) / 8;
            for (i, c) in patches.iter().enumerate() {
                page.rect(x + i as i32 * (pw + 6), y, pw, 110, *c);
            }
            for xx in 0..w {
                let g = (255.0 * xx as f32 / (w - 1) as f32).round() as u8;
                page.rect(x + xx, y + 130, 1, 70, [g, g, g]);
                page.rect(x + xx, y + 210, 1, 50, hue(xx as f32 / w as f32));
            }
            page.rect(x, y + 280, w, 164, [24, 24, 40]);
            let ty = y + 296;
            page.text(fonts, "Colour 24 px red", (24, Weight::Regular), (x + 16) as f32, (ty + 24) as f32, [255, 90, 90]);
            page.text(fonts, "Colour 24 px green", (24, Weight::Regular), (x + 250) as f32, (ty + 24) as f32, [90, 255, 120]);
            page.text(fonts, "Colour 24 px blue", (24, Weight::Regular), (x + 500) as f32, (ty + 24) as f32, [110, 140, 255]);
            page.text(
                fonts,
                "20 px yellow, 16 px cyan, 14 px magenta: The quick brown fox",
                (20, Weight::Regular),
                (x + 16) as f32,
                (ty + 70) as f32,
                [255, 230, 90],
            );
            page.text(fonts, "16 px cyan  Pack my box with five dozen liquor jugs", (16, Weight::Regular), (x + 16) as f32, (ty + 104) as f32, [90, 230, 240]);
            page.text(fonts, "14 px magenta  Sphinx of black quartz, judge my vow", (14, Weight::Regular), (x + 16) as f32, (ty + 130) as f32, [255, 120, 230]);
            page.rect(x, y + 460, w, 140, WHITE);
            page.lines(fonts, (x + 16) as f32, (y + 470) as f32, 6.0, BLACK, &SAMPLE[..4]);
        }
        Card::Saver => {
            // what the screensaver shows: a clock, a date, the weather and two pictures
            page.rect(x, y, w, h, [10, 10, 16]);
            page.text(fonts, "10:10", (220, Weight::Medium), (x + 28) as f32, (y + 220) as f32, [232, 232, 238]);
            page.text(fonts, "Thursday 29 October", (40, Weight::Regular), (x + 36) as f32, (y + 290) as f32, [190, 190, 202]);
            page.text(fonts, "18\u{b0}  Sunny, 21\u{b0} later", (28, Weight::Regular), (x + 36) as f32, (y + 336) as f32, [150, 150, 164]);
            page.rect(x + 36, y + 366, w - 72, 1, [60, 60, 72]);
            page.round(x + 36, y + 392, 330, 180, 24, |u, v| mix(mix([250, 170, 90], [120, 70, 150], u as f32 / 330.0), [30, 20, 60], v as f32 / 260.0));
            page.round(x + 392, y + 392, 330, 180, 24, |u, v| {
                mix(mix([60, 150, 190], [30, 60, 130], v as f32 / 180.0), [140, 210, 200], (u as f32 / 330.0) * 0.5)
            });
        }
        Card::Hues => {
            // saturated colours, skin tones and a sweep of hues: what grey takes away
            for xx in 0..w {
                page.rect(x + xx, y, 1, 150, hue(xx as f32 / w as f32));
            }
            let patches = [[230, 40, 40], [40, 200, 70], [50, 90, 240], [250, 210, 40], [30, 200, 220], [220, 60, 200]];
            let pw = (w - 5 * 8) / 6;
            for (i, c) in patches.iter().enumerate() {
                page.rect(x + i as i32 * (pw + 8), y + 172, pw, 120, *c);
            }
            let skins = [[255, 224, 196], [224, 172, 130], [141, 85, 54]];
            let sw = (w - 2 * 8) / 3;
            for (i, c) in skins.iter().enumerate() {
                page.rect(x + i as i32 * (sw + 8), y + 312, sw, 90, *c);
            }
            page.round(x, y + 422, w, 178, 24, |u, v| mix(mix([255, 120, 60], [80, 60, 200], u as f32 / w as f32), [20, 160, 120], v as f32 / 178.0));
            page.text(fonts, "Colour", (56, Weight::Medium), (x + 28) as f32, (y + 540) as f32, [255, 255, 255]);
        }
        Card::Window => {
            // a field of squares, so that the edge of an opening is easy to see, and the grey box where the window is expected: when the
            // window opens the box is replaced by what is behind the page
            for yy in (0..H as i32).step_by(60) {
                for xx in (0..W as i32).step_by(60) {
                    if (xx / 60 + yy / 60) % 2 == 0 {
                        page.rect(xx, yy, 60, 60, [26, 24, 38]);
                    }
                }
            }
            page.rect(x, y, w, h, [96, 96, 108]);
            let e = fonts.measure("Window", Type { px: 44, weight: Weight::Medium });
            page.text(
                fonts,
                "Window",
                (44, Weight::Medium),
                x as f32 + (w as f32 - e.width) / 2.0,
                y as f32 + (h as f32 + e.ascent - e.descent) / 2.0,
                [236, 236, 244],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(c: Card) -> Vec<u8> {
        let mut px = vec![0u8; BYTES];
        paint(c, &mut px, &mut Fonts::new());
        px
    }

    fn at(px: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * W + x) * 4;
        [px[i], px[i + 1], px[i + 2]]
    }

    fn lit(px: &[u8], x0: usize, y0: usize, w: usize, h: usize) -> usize {
        (y0..y0 + h).flat_map(|y| (x0..x0 + w).map(move |x| (x, y))).filter(|(x, y)| at(px, *x, *y) != at(px, x0, y0)).count()
    }

    #[test]
    fn every_card_is_a_whole_opaque_page() {
        for c in [Card::Text, Card::Lines, Card::Colour, Card::Saver, Card::Hues, Card::Window] {
            let px = card(c);
            assert!(px.chunks_exact(4).all(|p| p[3] == 255), "{c:?}: opaque everywhere");
            assert!(px.chunks_exact(4).any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0), "{c:?}: something is drawn");
        }
    }

    #[test]
    fn the_line_card_is_one_pixel_of_structure_in_four_tiles() {
        let px = card(Card::Lines);
        let z = Card::Lines.zone();
        let (x, y) = (z.x as usize, z.y as usize);
        // rows: white then black then white, a pixel each
        assert_eq!((at(&px, x + 5, y), at(&px, x + 5, y + 1), at(&px, x + 5, y + 2)), (WHITE, BLACK, WHITE));
        assert_eq!(at(&px, x + 5, y + 7), at(&px, x + 9, y + 7), "a row is the same along its length");
        // columns in the next tile
        let cx = x + 380 + 20;
        assert_eq!((at(&px, cx, y + 5), at(&px, cx + 1, y + 5), at(&px, cx + 2, y + 5)), (WHITE, BLACK, WHITE));
        assert_eq!(at(&px, cx + 4, y + 5), at(&px, cx + 4, y + 9), "a column is the same down its length");
        // the checkerboard alternates both ways
        let ty = y + 290 + 20;
        assert_eq!((at(&px, x, ty), at(&px, x + 1, ty), at(&px, x, ty + 1), at(&px, x + 1, ty + 1)), (WHITE, BLACK, BLACK, WHITE));
        // beyond the zone there is nothing
        assert_eq!(lit(&px, 900, 150, 600, 600), 0);
    }

    #[test]
    fn the_text_card_has_black_letters_on_white_and_white_letters_on_black_at_the_sizes_it_names() {
        let px = card(Card::Text);
        let z = Card::Text.zone();
        let (x, y) = (z.x as usize, z.y as usize);
        assert_eq!(at(&px, x + 2, y + 2), WHITE, "the upper block is white");
        assert!(lit(&px, x, y, 760, 290) > 1500, "with letters on it: {}", lit(&px, x, y, 760, 290));
        assert_eq!(at(&px, x + 2, y + 312), [18, 18, 18], "the lower block is near black");
        assert!(lit(&px, x, y + 310, 760, 290) > 1500);
        // the letters are the lines' own: the 24 px line is the lowest and its first row of ink is below the 12 px line's
        let first_ink = |y0: usize, y1: usize| (y0..y1).find(|yy| (x..x + 760).any(|xx| at(&px, xx, *yy) != WHITE));
        assert!(first_ink(y + 4, y + 40).is_some(), "the 12 px line");
        assert!(first_ink(y + 120, y + 290).is_some(), "the 24 px line");
        // nothing outside the zone
        assert_eq!(lit(&px, 900, 150, 600, 600), 0);
    }

    #[test]
    fn the_colour_card_has_a_ramp_that_climbs_and_a_sweep_of_hues() {
        let px = card(Card::Colour);
        let z = Card::Colour.zone();
        let (x, y) = (z.x as usize, z.y as usize);
        let ramp: Vec<u8> = (0..760).step_by(40).map(|xx| at(&px, x + xx, y + 160)[0]).collect();
        assert!(ramp.windows(2).all(|w| w[0] < w[1]), "{ramp:?}");
        assert_eq!((ramp[0], *ramp.last().unwrap() > 240), (0, true));
        assert_eq!(at(&px, x + 5, y + 5), [255, 0, 0]);
        assert_eq!(at(&px, x + 5 + 90, y + 5), [0, 255, 0]);
        let sweep: Vec<[u8; 3]> = (0..760).step_by(190).map(|xx| at(&px, x + xx, y + 230)).collect();
        assert_ne!(sweep[0], sweep[1]);
        assert_ne!(sweep[1], sweep[2]);
    }

    #[test]
    fn the_saver_card_has_a_clock_a_date_and_two_pictures() {
        let px = card(Card::Saver);
        let z = Card::Saver.zone();
        let (x, y) = (z.x as usize, z.y as usize);
        assert_eq!(at(&px, x + 4, y + 4), [10, 10, 16]);
        assert!(lit(&px, x + 28, y + 40, 420, 190) > 2500, "the clock: {}", lit(&px, x + 28, y + 40, 420, 190));
        assert!(lit(&px, x + 36, y + 255, 420, 45) > 300, "the date");
        let (a, b) = (at(&px, x + 36 + 165, y + 392 + 90), at(&px, x + 392 + 165, y + 392 + 90));
        assert!(a[0] > 100 && a[0] > a[2] && b[2] > 120 && b[2] > b[0] && a != b, "a warm picture and a cool one: {a:?} {b:?}");
    }

    #[test]
    fn the_hue_card_has_the_saturated_colours_grey_would_take_away() {
        let px = card(Card::Hues);
        let z = Card::Hues.zone();
        let (x, y) = (z.x as usize, z.y as usize);
        let red = at(&px, x + 3, y + 3);
        assert!(red[0] == 255 && red[1] < 20 && red[2] == 0, "the sweep starts at red: {red:?}");
        assert_eq!(at(&px, x + 10, y + 180), [230, 40, 40]);
        assert_eq!(at(&px, x + 10, y + 330), [255, 224, 196]);
        let saturated = (0..760)
            .step_by(20)
            .filter(|xx| {
                let p = at(&px, x + xx, y + 50);
                p.iter().max().unwrap() - p.iter().min().unwrap() > 200
            })
            .count();
        assert!(saturated >= 36, "{saturated}");
    }

    #[test]
    fn the_window_card_marks_where_the_window_is_expected_and_has_edges_to_see_it_by() {
        let px = card(Card::Window);
        let z = Card::Window.zone();
        let (x, y) = (z.x as usize, z.y as usize);
        assert_eq!(at(&px, x + 4, y + 4), [96, 96, 108], "the box");
        assert_ne!(at(&px, x - 1, y - 1), at(&px, x + 4, y + 4), "the box ends where the window is");
        assert!(lit(&px, x + 100, y + 100, 280, 70) > 200, "its word");
        // the squares: dark and black alternate every 60 px, over the whole page
        assert_eq!((at(&px, 5, 5), at(&px, 65, 5), at(&px, 5, 65), at(&px, 65, 65)), ([26, 24, 38], BLACK, BLACK, [26, 24, 38]));
        assert_eq!(at(&px, 1900, 1060), [26, 24, 38], "and to the corner");
    }

    /// `LAB_CARDS=<dir> cargo test -p lumatv-ui --lib lab::card::tests::the_cards_as_pictures -- --ignored` writes each card as a PNG to look at.
    #[test]
    #[ignore]
    fn the_cards_as_pictures() {
        let Some(dir) = std::env::var_os("LAB_CARDS") else { return };
        std::fs::create_dir_all(&dir).unwrap();
        for c in [Card::Text, Card::Lines, Card::Colour, Card::Saver, Card::Hues, Card::Window] {
            let px = card(c);
            let file = std::fs::File::create(std::path::Path::new(&dir).join(format!("{c:?}.png").to_lowercase())).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&px).unwrap();
        }
    }
}
