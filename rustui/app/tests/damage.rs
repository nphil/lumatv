//! Drawing only what changed, through the real renderer on a software GL: every frame of LumaHome drawn into a page of a ring of
//! three, only where the page differs from what it holds, is the picture the same frame is drawn whole.
//!
//! The host draws each frame twice (`HostConfig::reference`): into the page the ring gives it, as little as it can, and whole into
//! a target of its own. The pictures are compared pixel for pixel, which is the whole claim: drawing less must never show.

use lumatv_render::host::FrameCost;
use lumatv_rustui::rig::{Rig, RigConfig};

fn assert_same(rig: &Rig, cost: &FrameCost, frame: usize, what: &str) {
    assert_eq!(cost.mismatch_px, 0, "{what}, frame {frame}: {} px differ from the frame drawn whole, first {:?}", cost.mismatch_px, rig.engine.mismatch());
}

#[test]
fn lumahome_drawn_as_it_is_owed_is_the_frame_drawn_whole_over_30_frames() {
    let mut rig = Rig::new(RigConfig { pages: 3, reference: true, gpu_sync: false }).expect("a software GL and LumaHome");
    let (mut frames, mut partial, mut drew, mut damaged) = (0usize, 0usize, 0u64, 0u64);
    // A second of nothing but the wallpaper's steps and the clock's hand, then the gate's D-pad and sweep scripts until 30 frames.
    for (script, seconds) in [("idle", 1.0), ("mix", 3.0)] {
        let mut stop = false;
        let mut results = Vec::new();
        rig.run(script, seconds, |cost| {
            if frames < 30 {
                results.push(*cost);
                frames += 1;
            } else {
                stop = true;
            }
        })
        .expect("frames");
        for (n, cost) in results.iter().enumerate() {
            assert_same(&rig, cost, n, script);
            partial += usize::from(cost.region_px < 1920 * 1080);
            drew += cost.region_px;
            damaged += cost.damage_px;
        }
        if stop || frames >= 30 {
            break;
        }
    }
    assert!(frames >= 30, "{frames} frames drawn");
    assert!(partial > 15, "only {partial} of {frames} frames were drawn in part");
    eprintln!(
        "{frames} frames, {partial} drawn in part, {:.1} % of the panel drawn on average, {:.1} % changed",
        100.0 * drew as f64 / (frames as f64 * 2_073_600.0),
        100.0 * damaged as f64 / (frames as f64 * 2_073_600.0)
    );
}

mod stage {
    use compositor_protocol::{Backdrop, Fill, Glass, Props, Rgba, Shadow, Shape, Sheen};
    use lumatv_rustui::rig::{RigConfig, Stage};

    fn solid(radius: f32, r: u8, g: u8, b: u8) -> Shape {
        Shape { corner_radius: radius, fill: Fill::Solid(Rgba(r, g, b, 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) }
    }

    fn glass(backdrop: Backdrop) -> Glass {
        Glass {
            corner_radius: 24.0,
            tint: Rgba(10, 8, 16, 90),
            edge: Rgba(30, 30, 30, 30),
            edge_width: 1.0,
            highlight: Rgba(12, 12, 12, 12),
            sheen: Some(Sheen { peak: 0.16, floor: 0.05, start: 0.38, end: 0.6 }),
            shadows: [Some(Shadow { offset_y: 12.0, blur: 30.0, colour: Rgba(0, 0, 0, 70) }), None],
            depth: None,
            backdrop,
            saturation: 1.2,
            refraction: 0.0,
            edge_lens: None,
            tint_gradient: None,
        }
    }

    /// Layers that move under, beside and far from glass that blurs what is below it afresh each frame (a live backdrop) and glass that
    /// shows the blur of what was there when it last changed (a cached one), a picture the app changes in places, and layers that come
    /// and go: every frame drawn into a page of the ring is the frame drawn whole.
    #[test]
    fn glass_that_blurs_what_moves_near_it_is_drawn_as_it_is_owed() {
        let mut stage = Stage::new(RigConfig { pages: 3, reference: true, gpu_sync: false }).expect("a software GL");
        let _back = stage.ui.scene.shape(1920, 1080, 0, solid(0.0, 24, 28, 44), Props::at(0.0, 0.0));
        let stripes: Vec<_> = (0..16)
            .map(|i| stage.ui.scene.shape(60, 1080, 1, solid(0.0, 40 + 12 * i as u8, 90, 200 - 9 * i as u8), Props::at(120.0 * i as f32, 0.0)))
            .collect();
        let bar_a = stage.ui.scene.shape(90, 90, 2, solid(20.0, 240, 150, 40), Props::at(0.0, 150.0));
        let bar_b = stage.ui.scene.shape(90, 90, 2, solid(20.0, 80, 220, 120), Props::at(700.0, 20.0));
        let bar_c = stage.ui.scene.shape(90, 90, 2, solid(20.0, 220, 60, 90), Props::at(0.0, 800.0));
        let _live_top = stage.ui.scene.glass(560, 280, 5, glass(Backdrop::Live { radius: 20.0, downsample: 2 }), Props::at(680.0, 120.0));
        let live_low = stage.ui.scene.glass(500, 240, 6, glass(Backdrop::Live { radius: 12.0, downsample: 4 }), Props::at(900.0, 700.0));
        let cached = stage.ui.scene.glass(480, 220, 7, glass(Backdrop::Cached { radius: 24.0 }), Props::at(80.0, 620.0));
        let mut picture = stage.ui.scene.image(200, 120, 8, Props::at(1500.0, 600.0)).expect("a picture");
        let mut toast = None;
        let (mut frames, mut partial) = (0, 0);
        for k in 0..80usize {
            let f = k as f32;
            bar_a.set(Props::at((f * 13.0) % 1800.0, 150.0));
            bar_b.set(Props::at(660.0 + (f * 7.0) % 700.0, 20.0));
            bar_c.set(Props::at((f * 17.0) % 1800.0, 800.0));
            // a picture the app changes in places: a square that runs along it
            let at = (k % 160) as u32;
            let pixels = picture.pixels_mut();
            pixels.fill(0);
            for y in 20..60u32 {
                for x in at..at + 40 {
                    let i = ((y * 200 + x) * 4) as usize;
                    pixels[i..i + 4].copy_from_slice(&[200, 90, 30, 255]);
                }
            }
            picture.damage(None);
            if k % 40 == 20 {
                // a restyle of one stripe changes what the cached glass was a picture of
                stripes[(k / 40) % stripes.len()].reshape(solid(0.0, 200, 40 + k as u8 % 100, 60));
            }
            if k % 70 == 35 {
                cached.set(Props::at(80.0 + 40.0 * f % 600.0, 620.0));
            }
            if k == 60 {
                toast = Some(stage.ui.scene.shape(420, 90, 20, solid(30.0, 240, 240, 250), Props { opacity: 0.0, ..Props::at(750.0, 40.0) }));
            }
            if let (62, Some(toast)) = (k, &toast) {
                toast.animate(Props::at(750.0, 40.0), 200, compositor_protocol::Ease::Out);
            }
            if k == 75 {
                toast = None;
            }
            live_low.set(Props::at(900.0 + (f * 3.0) % 300.0, 700.0));
            let cost = stage.frame().expect("a frame");
            assert_eq!(cost.mismatch_px, 0, "frame {k}: {} px differ from the frame drawn whole, first {:?}", cost.mismatch_px, stage.engine.mismatch());
            frames += 1;
            partial += usize::from(cost.region_px < 1920 * 1080);
        }
        assert_eq!(frames, 80);
        assert!(partial > 40, "only {partial} of {frames} frames were drawn in part");
        eprintln!("{partial} of {frames} frames drawn in part");
    }
}
