//! Renders the weather pictures on the host: `cargo run --release --example weather_art -- <out dir> [light] [view]` writes a contact sheet of every
//! condition for the card's sky (still, then moments of its motion) and, with `view`, the view's large sky of a few of them, each over the glass
//! and the sky's wash as the shell lays them, from the same poses the compositor is given.

use lumatv_ui::home::weather_art::{Cond, Sky, Stage, compose, parts};
use lumatv_ui::home::weather_sprites::Pal;

fn png(path: &str, w: u32, h: u32, rgba: &[u8]) {
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// The sky's wash over the ground: its colour at the top, fading to nothing at the foot.
fn washed(cond: Cond, light: bool, ground: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    let (rgb, a) = cond.wash(&Pal::standard(light), light);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let f = a * (1.0 - y as f32 / h as f32);
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                out[o + c] = (rgb[c] * 255.0 * f + ground[c] as f32 * (1.0 - f)).round() as u8;
            }
            out[o + 3] = 255;
        }
    }
    out
}

/// The picture over a wash, by compositing the picture on transparent and then that over the wash.
fn frame(cond: Cond, st: Stage, light: bool, t: Option<f64>, ground: [u8; 4]) -> Vec<u8> {
    let (w, h) = (st.w as u32, st.h as u32);
    let p = parts(cond, st, light, Pal::standard(light));
    let mut img = washed(cond, light, ground, w, h);
    // compose draws over `ground` pixels it is given: give it the wash itself
    let base = img.clone();
    let layer = compose(&p, w, h, t, [0, 0, 0, 0]);
    for i in 0..(w * h) as usize {
        let a = layer[i * 4 + 3] as f32 / 255.0;
        for c in 0..3 {
            img[i * 4 + c] = (layer[i * 4 + c] as f32 + base[i * 4 + c] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
        }
    }
    img
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "/tmp/weather-art".into());
    let light = args.iter().any(|s| s == "light");
    let view = args.iter().any(|s| s == "view");
    std::fs::create_dir_all(&out).unwrap();
    let ground = if light { [250, 244, 237, 255] } else { [38, 35, 58, 255] };
    let c = |sky, day, heavy, wind| Cond { sky, day, heavy, wind };
    let list = [
        ("clear-day", c(Sky::Clear, true, 0, false)),
        ("clear-night", c(Sky::Clear, false, 0, false)),
        ("partly-day", c(Sky::Partly, true, 1, false)),
        ("partly-night", c(Sky::Partly, false, 1, false)),
        ("mostly-clear-day", c(Sky::Partly, true, 0, false)),
        ("cloudy", c(Sky::Cloudy, true, 1, false)),
        ("fog", c(Sky::Fog, true, 1, false)),
        ("drizzle", c(Sky::Drizzle, true, 1, false)),
        ("rain", c(Sky::Rain, true, 1, false)),
        ("rain-heavy", c(Sky::Rain, true, 2, false)),
        ("storm", c(Sky::Storm, true, 1, false)),
        ("snow", c(Sky::Snow, true, 1, false)),
        ("snow-heavy", c(Sky::Snow, false, 2, false)),
        ("windy-clear", c(Sky::Clear, true, 0, true)),
        ("windy-rain", c(Sky::Rain, true, 1, true)),
    ];
    // the card's sky: the hero on the right, where the temperature is not
    let card = Stage { w: 433.0, h: 244.0, hero: (318.0, 104.0), size: 190.0, ground: 236.0, fade: [14.0, 14.0, 10.0, 30.0], shy: 0.5, card: None };
    let times: [Option<f64>; 4] = [None, Some(3_000.0), Some(9_400.0), Some(21_000.0)];
    let cols = 3usize;
    let rows = list.len().div_ceil(cols);
    for (ti, t) in times.iter().enumerate() {
        let (w, h) = (card.w as u32 * cols as u32, card.h as u32 * rows as u32);
        let mut sheet = vec![0u8; (w * h * 4) as usize];
        for (i, (_, cond)) in list.iter().enumerate() {
            let img = frame(*cond, card, light, *t, ground);
            let (ox, oy) = ((i % cols) as u32 * card.w as u32, (i / cols) as u32 * card.h as u32);
            for y in 0..card.h as u32 {
                let dst = (((oy + y) * w + ox) * 4) as usize;
                let n = card.w as usize * 4;
                sheet[dst..dst + n].copy_from_slice(&img[(y as usize * n)..((y as usize + 1) * n)]);
            }
        }
        png(&format!("{out}/card-{ti}.png"), w, h, &sheet);
    }
    // a strip of frames, 0.7 s apart, of the conditions that move the most
    for name in ["clear-day", "clear-night", "partly-day", "rain", "storm", "snow", "windy-rain"] {
        let cond = list.iter().find(|(n, _)| *n == name).unwrap().1;
        let frames = 8u32;
        let (w, h) = (card.w as u32, card.h as u32);
        let mut strip = vec![0u8; (w * frames * h * 4) as usize];
        for f in 0..frames {
            let img = frame(cond, card, light, Some(30_000.0 + f as f64 * 700.0), ground);
            for y in 0..h {
                let row = ((y * frames + f) * w) as usize * 4;
                strip[row..row + (w * 4) as usize].copy_from_slice(&img[(y * w * 4) as usize..((y + 1) * w * 4) as usize]);
            }
        }
        // the frames side by side: 8 across
        let mut wide = vec![0u8; strip.len()];
        for f in 0..frames {
            for y in 0..h {
                let src = ((y * frames + f) * w) as usize * 4;
                let dst = (y * frames * w + f * w) as usize * 4;
                wide[dst..dst + (w * 4) as usize].copy_from_slice(&strip[src..src + (w * 4) as usize]);
            }
        }
        png(&format!("{out}/strip-{name}.png"), w * frames, h, &wide);
    }
    if view {
        let st = Stage { w: 1160.0, h: 620.0, hero: (560.0, 276.0), size: 470.0, ground: 600.0, fade: [150.0, 150.0, 50.0, 150.0], shy: 0.2, card: None };
        for name in ["clear-day", "clear-night", "partly-day", "cloudy", "rain", "storm", "snow", "fog"] {
            let cond = list.iter().find(|(n, _)| *n == name).unwrap().1;
            let img = frame(cond, st, light, Some(14_000.0), ground);
            png(&format!("{out}/view-{name}.png"), st.w as u32, st.h as u32, &img);
        }
    }
    println!("wrote {out}");
}
