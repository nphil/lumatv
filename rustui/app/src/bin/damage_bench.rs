//! What drawing only what changed saves, on the software rasteriser: the gate's idle and D-pad scripts of LumaHome drawn in full every
//! frame and drawn into a ring of pages where each owes only what changed, with the pixels each frame drew and the time it took.
//!
//!   lumatv-damage-bench [--reps N] [--seconds idle=10,dpad=7,sweep=7] [--pages 3]
//!
//! The runs alternate between the two ways (so a busy machine hits both), each on a fresh rig; the first frames of a run (the ring's
//! first turn, which draws every page whole, and the first uploads) are left out. Time is until the rasteriser has finished the frame
//! (`HostConfig::gpu_sync`), which on llvmpipe is the frame's work: it says how the work scales with the pixels, not what the C4's GPU
//! takes (that is `docs/research/rustui-result.md`'s estimate from the pixel counts, and `tools/tv-damage.py` on the set).

use lumatv_render::host::FrameCost;
use lumatv_rustui::rig::{Rig, RigConfig};
use std::collections::BTreeMap;

const PANEL_PX: f64 = 1920.0 * 1080.0;
/// Frames left out at the start of a run.
const COLD: usize = 8;

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

fn column(frames: &[FrameCost], f: impl Fn(&FrameCost) -> f64) -> Vec<f64> {
    let mut v: Vec<f64> = frames.iter().map(f).collect();
    v.sort_by(f64::total_cmp);
    v
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn main() {
    let mut reps = 3;
    let mut pages = 3;
    let mut seconds: Vec<(String, f64)> = vec![("idle".into(), 10.0), ("dpad".into(), 7.0), ("sweep".into(), 7.0)];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--reps" => reps = args.next().and_then(|v| v.parse().ok()).expect("--reps N"),
            "--pages" => pages = args.next().and_then(|v| v.parse().ok()).expect("--pages N"),
            "--seconds" => {
                seconds = args
                    .next()
                    .expect("--seconds idle=10,dpad=7")
                    .split(',')
                    .map(|p| {
                        let (n, s) = p.split_once('=').expect("name=seconds");
                        (n.to_owned(), s.parse().expect("seconds"))
                    })
                    .collect()
            }
            other => panic!("unknown option {other}"),
        }
    }
    // (scenario, ways) -> the frames of every run
    let mut all: BTreeMap<(usize, usize), Vec<FrameCost>> = BTreeMap::new();
    let ways = [("whole", 0usize), ("ring", pages)];
    for rep in 0..reps {
        for (s, (name, secs)) in seconds.iter().enumerate() {
            // A, B, B, A over the repetitions: whichever of the two runs first or last is not always the same one.
            let order: [usize; 2] = if rep % 2 == 0 { [0, 1] } else { [1, 0] };
            for w in order {
                let mut rig = Rig::new(RigConfig { pages: ways[w].1, reference: false, gpu_sync: true }).expect("a software GL and LumaHome");
                // the ring's first turn and the first uploads are not the steady state
                rig.run("idle", 1.0, |_| {}).expect("settling");
                let mut frames = Vec::new();
                rig.run(name, *secs, |cost| frames.push(*cost)).expect("frames");
                all.entry((s, w)).or_default().extend(frames.into_iter().skip(COLD));
            }
        }
        eprintln!("repetition {} of {reps} done", rep + 1);
    }
    println!(
        "| script | frames drawn | how | drawn px / frame (mean, p50, max) | of the panel | changed px / frame (mean) | passes | layers drawn | render ms p50 / p90 / max |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    let mut summary = Vec::new();
    for (s, (name, _)) in seconds.iter().enumerate() {
        for (w, (how, _)) in ways.iter().enumerate() {
            let frames = &all[&(s, w)];
            let drawn = column(frames, |f| f.region_px as f64);
            let changed = column(frames, |f| f.damage_px as f64);
            let passes = column(frames, |f| f64::from(f.rects));
            let layers = column(frames, |f| f64::from(f.layers));
            let ms = column(frames, |f| f64::from(f.gpu_us) / 1000.0);
            println!(
                "| {name} | {} | {how} | {:.0} / {:.0} / {:.0} | {:.1} % | {:.0} | {:.1} | {:.1} | {:.2} / {:.2} / {:.2} |",
                frames.len(),
                mean(&drawn),
                percentile(&drawn, 0.5),
                drawn.last().copied().unwrap_or(0.0),
                100.0 * mean(&drawn) / PANEL_PX,
                mean(&changed),
                mean(&passes),
                mean(&layers),
                percentile(&ms, 0.5),
                percentile(&ms, 0.9),
                ms.last().copied().unwrap_or(0.0),
            );
            summary.push((name.clone(), *how, mean(&drawn), percentile(&ms, 0.5), mean(&ms), mean(&column(frames, |f| f64::from(f.draw_calls)))));
        }
    }
    println!();
    println!("| script | how | place us | plan us | damage us | backdrops us | draw us | CPU to issue us | to the rasteriser's end us (p50 of each) |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for (s, (name, _)) in seconds.iter().enumerate() {
        for (w, (how, _)) in ways.iter().enumerate() {
            let frames = &all[&(s, w)];
            let p50 = |f: fn(&FrameCost) -> f64| percentile(&column(frames, f), 0.5);
            println!(
                "| {name} | {how} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
                p50(|f| f64::from(f.place_us)),
                p50(|f| f64::from(f.plan_us)),
                p50(|f| f64::from(f.damage_us)),
                p50(|f| f64::from(f.backdrop_us)),
                p50(|f| f64::from(f.draw_us)),
                p50(|f| f64::from(f.cpu_us)),
                p50(|f| f64::from(f.gpu_us)),
            );
        }
    }
    println!();
    println!("| script | pixels drawn, ring / whole | render time p50, ring / whole | render time mean, ring / whole | draw calls / frame, ring / whole |");
    println!("|---|---|---|---|---|");
    for pair in summary.chunks(2) {
        let (whole, ring) = (&pair[0], &pair[1]);
        println!(
            "| {} | {:.1} % | {:.1} % | {:.1} % | {:.0} / {:.0} |",
            whole.0,
            100.0 * ring.2 / whole.2,
            100.0 * ring.3 / whole.3,
            100.0 * ring.4 / whole.4,
            ring.5,
            whole.5
        );
    }
}
