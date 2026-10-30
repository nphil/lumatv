//! Renders the weather card's pictures on the host: `cargo run --example weather_art -- <out dir> [side] [light]` writes a contact sheet of every
//! condition (still, then four moments of its motion) and a strip of frames for one condition, from the same poses the compositor is given.

use lumatv_ui::home::weather_art::{Cond, Sky, compose, parts};

fn png(path: &str, w: u32, h: u32, rgba: &[u8]) {
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "/tmp/weather-art".into());
    let side: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(180);
    let light = args.get(3).is_some_and(|s| s == "light");
    std::fs::create_dir_all(&out).unwrap();
    let ground = if light { [250, 244, 237, 255] } else { [38, 35, 58, 255] };
    let c = |sky, day, heavy, wind| Cond { sky, day, heavy, wind };
    let list = [
        ("clear-day", c(Sky::Clear, true, 0, false)),
        ("clear-night", c(Sky::Clear, false, 0, false)),
        ("mostly-clear-day", c(Sky::Partly, true, 0, false)),
        ("partly-day", c(Sky::Partly, true, 1, false)),
        ("partly-night", c(Sky::Partly, false, 1, false)),
        ("cloudy", c(Sky::Cloudy, true, 1, false)),
        ("fog", c(Sky::Fog, true, 1, false)),
        ("drizzle", c(Sky::Drizzle, true, 1, false)),
        ("rain-light", c(Sky::Rain, true, 0, false)),
        ("rain", c(Sky::Rain, true, 1, false)),
        ("rain-heavy", c(Sky::Rain, true, 2, false)),
        ("storm", c(Sky::Storm, true, 1, false)),
        ("snow-light", c(Sky::Snow, true, 0, false)),
        ("snow", c(Sky::Snow, true, 1, false)),
        ("snow-heavy", c(Sky::Snow, true, 2, false)),
        ("windy-clear", c(Sky::Clear, true, 0, true)),
        ("windy-cloudy", c(Sky::Cloudy, true, 1, true)),
    ];
    let times: [Option<f64>; 5] = [None, Some(1_000.0), Some(3_600.0), Some(6_200.0), Some(9_000.0)];
    let cols = list.len().min(6);
    let rows = list.len().div_ceil(cols);
    // one sheet per moment, every condition on it
    for (ti, t) in times.iter().enumerate() {
        let (w, h) = (side * cols as u32, side * rows as u32);
        let mut sheet = vec![0u8; (w * h * 4) as usize];
        for (i, (_, cond)) in list.iter().enumerate() {
            let p = parts(*cond, side as f32, light);
            let img = compose(&p, side, side, *t, ground);
            let (ox, oy) = ((i % cols) as u32 * side, (i / cols) as u32 * side);
            for y in 0..side {
                let dst = (((oy + y) * w + ox) * 4) as usize;
                sheet[dst..dst + (side * 4) as usize].copy_from_slice(&img[(y * side * 4) as usize..((y + 1) * side * 4) as usize]);
            }
        }
        png(&format!("{out}/sheet-{ti}.png"), w, h, &sheet);
    }
    // a strip of 12 frames, 0.5 s apart, of the conditions that move the most
    for name in ["clear-day", "clear-night", "partly-day", "rain", "storm", "snow", "fog", "windy-cloudy"] {
        let cond = list.iter().find(|(n, _)| *n == name).unwrap().1;
        let p = parts(cond, side as f32, light);
        let frames = 12u32;
        let mut strip = vec![0u8; (side * frames * side * 4) as usize];
        for f in 0..frames {
            let img = compose(&p, side, side, Some(if name == "storm" { 25_000.0 } else { 20_000.0 } + f as f64 * 500.0), ground);
            for y in 0..side {
                let row = (y * frames * side + f * side) as usize * 4;
                strip[row..row + (side * 4) as usize].copy_from_slice(&img[(y * side * 4) as usize..((y + 1) * side * 4) as usize]);
            }
        }
        png(&format!("{out}/strip-{name}.png"), side * frames, side, &strip);
    }
    println!("wrote {out}");
}
