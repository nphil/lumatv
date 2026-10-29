//! A host-installed sink receives real renderer events instead of the standalone stderr path.
use std::sync::atomic::{AtomicBool, Ordering};

use device_profile::Rotation;
use lumatv_render::{Gles, Panel, Renderer, set_log_sink};

static COMPILED: AtomicBool = AtomicBool::new(false);

#[test]
fn renderer_construction_sends_its_events_to_the_hosts_sink() {
    fn sink(message: &dyn std::fmt::Display) {
        if message.to_string().starts_with("shader programs compiled and linked in ") {
            COMPILED.store(true, Ordering::Relaxed);
        }
    }
    set_log_sink(sink).unwrap();
    let gl = Gles::software(c"libEGL.so.1").expect("Mesa software GL is required");
    let panel = Panel { rotation: Rotation::R0, logical_width: 64.0, logical_height: 64.0, width: 64, height: 64 };
    let _renderer = Renderer::new(gl, panel).unwrap();
    assert!(COMPILED.load(Ordering::Relaxed), "the renderer event did not reach its host sink");
    assert!(set_log_sink(sink).is_err(), "an installed host sink cannot be silently replaced");
}
