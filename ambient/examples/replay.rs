//! Replays a probe dump (`ambient-probe -d FILE`) through the engine and prints what eight lights around the set
//! would show, so defaults can be tuned against frames recorded on the TV without touching the TV.
//!
//!   cargo run --example replay -- probe.dump [delay_ms]

use lumatv_ambient::engine::{Config, Engine};
use lumatv_ambient::layout::{Light, Target};
use lumatv_ambient::zones::{FrameParams, ZoneFrame, parse_dump};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: replay DUMP [delay_ms]");
    let delay_ms: u32 = args.next().map_or(0, |a| a.parse().expect("delay_ms"));
    let bytes = std::fs::read(&path).expect("read the dump");
    let frames = parse_dump(&bytes).expect("a VTH1 dump");

    // Hue-style backlights: corners and edge middles, just outside the picture.
    let spots: [(f32, f32); 8] = [
        (-0.08, -0.08),
        (0.5, -0.1),
        (1.08, -0.08),
        (1.1, 0.5),
        (1.08, 1.08),
        (0.5, 1.1),
        (-0.08, 1.08),
        (-0.1, 0.5),
    ];
    let lights: Vec<Light> = spots
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| Light {
            target: Target::Hue { channel: i as u8 },
            x,
            y,
        })
        .collect();
    let mut engine = Engine::new(
        Config {
            delay_ms,
            ..Config::default()
        },
        lights,
    )
    .expect("config");

    let params = FrameParams::default();
    let t0 = frames.first().map_or(0, |f| f.t_ms);
    for (i, f) in frames.iter().enumerate() {
        let zf = ZoneFrame::from_histograms(f.preset, &f.zones, params).expect("a full frame");
        let t = u64::from(f.t_ms - t0);
        let next = frames
            .get(i + 1)
            .map_or(t + 2000, |n| u64::from(n.t_ms - t0));
        engine.push_frame(t, zf);
        println!(
            "-- capture {} preset {:?} at {} ms, wanted preset {:?}, bars {:?}",
            i,
            f.preset,
            t,
            engine.wanted_preset(),
            engine.bars()
        );
        let mut now = t;
        while now < next.min(t + 1000) {
            let out = engine.tick(now).to_vec();
            let line: Vec<String> = out
                .iter()
                .map(|c| {
                    format!(
                        "{:02x}{:02x}{:02x}",
                        c.to_u8()[0],
                        c.to_u8()[1],
                        c.to_u8()[2]
                    )
                })
                .collect();
            println!("{now:>7} ms  {}", line.join(" "));
            now += 100;
        }
    }
}
