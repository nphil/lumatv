#[path = "../src/backdrop/plan.rs"]
mod backdrop_plan;

use compositor_protocol::LayerId;
use backdrop_plan::{plan, Ask, Layer, Refresh};

type Area = [i32; 4];

fn plain(id: u32, area: Area) -> Layer {
    Layer { id: LayerId(id), version: 1, now: Some(area), rest: Some(area), ask: None }
}

fn glass(id: u32, body: Area, samples: Area) -> Layer {
    Layer {
        ask: Some(Ask { refresh: Refresh::Live, radius: 10.0, downsample: 4, body_now: body, body_rest: body, sample_now: samples, sample_rest: samples }),
        ..plain(id, body)
    }
}

#[test]
fn live_lens_samples_are_padded_and_clipped_to_panel() {
    let panel = [0, 0, 300, 200];
    let layers = [glass(1, [10, 20, 80, 100], [-14, -4, 104, 124])];
    let result = plan(&layers, panel, |_, _| 10);
    assert_eq!(result[0].area, [0, 0, 114, 134]);
}

#[test]
fn live_lens_intervening_sample_prevents_sharing() {
    let panel = [0, 0, 400, 200];
    let layers = [
        glass(1, [10, 20, 80, 100], [10, 20, 80, 100]),
        plain(2, [166, 30, 175, 60]),
        glass(3, [190, 20, 260, 100], [166, -4, 284, 124]),
    ];
    let result = plan(&layers, panel, |_, _| 10);
    assert_eq!(result.len(), 2);
    assert_eq!(result[1].upto, 2);
    assert_eq!(result[1].area, [156, 0, 294, 134]);
}
