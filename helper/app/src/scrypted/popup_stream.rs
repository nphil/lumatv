//! Which stream a camera pop-up plays in a hardware video window.
//!
//! The kernel re-times the panel to the rate of the decoder that feeds the main window: a 1080p stream at 25 pictures a second takes a 120 Hz panel to
//! 100 Hz, and a rate request does not bring it back (`docs/research/compositor/slot-2026-10-30-r23`). On the set the panel kept 120 Hz for 640x480 at 10,
//! 1280x720 declared at 25 and streams declared at 20. So a pop-up's window takes only a *calm* stream, one of those two kinds, and asks the bridge for it by
//! limits (`accept`: the bridge never picks a stream outside them): the camera's low stream (the bridge's `light`: Scrypted's low-resolution destination,
//! else the smallest stream that fits) inside 1280x720 at up to 30, else inside 1920x1088 at up to 20. A camera with no such stream has its pop-up drawn
//! as a texture (the compositor's CPU path, which the panel never sees); the full view keeps the camera's own stream and is not decided here.

use super::bridge::{Camera, Dial, Trouble};
use serde_json::{Value, json};

/// A picture this size or smaller is calm at any rate up to [`SMALL_MAX_FPS`] (720p at 25 kept 120 Hz on the set).
const SMALL_MAX_W: u32 = 1280;
const SMALL_MAX_H: u32 = 720;
const SMALL_MAX_FPS: f64 = 30.0;
/// A stream this slow is calm up to the sub window's size (20 a second kept 120 Hz on the set).
const SLOW_MAX_W: u32 = 1920;
const SLOW_MAX_H: u32 = 1088;
const SLOW_MAX_FPS: f64 = 20.0;

/// The code of a refusal when no stream of the camera is calm.
pub const NO_CALM: &str = "no_calm_stream";

/// Whether a stream of this size and rate leaves the panel's rate alone. A size or a rate of 0 is not known: an unknown size is never small, an unknown rate is
/// never slow.
pub fn calm(width: u32, height: u32, fps: f64) -> bool {
    let small = width > 0 && height > 0 && width <= SMALL_MAX_W && height <= SMALL_MAX_H && fps <= SMALL_MAX_FPS;
    let slow = fps > 0.0 && fps <= SLOW_MAX_FPS && width <= SLOW_MAX_W && height <= SLOW_MAX_H;
    small || slow
}

/// A codec the window plays (an unnamed one is the bridge's to judge).
fn codec_ok(codec: &str) -> bool {
    matches!(codec, "" | "h264" | "h265")
}

/// Whether the camera has a stream a window can play calmly, by what the bridge last said of its streams. No description is no: the pop-up is then a texture,
/// which cannot touch the panel whatever the stream turns out to be.
pub fn window_ok(camera: &Camera) -> bool {
    camera
        .streams
        .iter()
        .chain(camera.native.iter())
        .any(|s| codec_ok(&s.codec) && calm(s.width, s.height, s.fps))
}

/// The two kinds of calm stream, asked for in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Up to 1280x720 at 30.
    Small,
    /// Up to 1920x1088 at 20.
    Slow,
}

pub const STEPS: [Step; 2] = [Step::Small, Step::Slow];

impl Step {
    /// What the bridge is told the client takes.
    pub fn accept(self) -> Value {
        let (w, h, fps) = match self {
            Step::Small => (SMALL_MAX_W, SMALL_MAX_H, SMALL_MAX_FPS as u32),
            Step::Slow => (SLOW_MAX_W, SLOW_MAX_H, SLOW_MAX_FPS as u32),
        };
        json!({"codecs": ["h264", "h265"], "max_width": w, "max_height": h, "max_fps": fps})
    }

    pub fn words(self) -> &'static str {
        match self {
            Step::Small => "up to 1280x720",
            Step::Slow => "up to 1920x1088 at 20 a second",
        }
    }
}

fn nothing_calm() -> Trouble {
    Trouble::Bridge { status: 501, code: NO_CALM.to_owned() }
}

/// The camera's low stream for a window: `ask` is the bridge's live call for the stream inside a step's limits. The steps are tried in order; a step with no
/// stream (the bridge's `501`) or an answer that is not calm after all (a bridge that does not know `accept`) goes on to the next; any other refusal ends it.
/// With nothing calm the answer is [`NO_CALM`]. Each step's outcome is logged, the chosen stream last.
pub fn ask_window(camera: &str, mut ask: impl FnMut(Step) -> Result<Dial, Trouble>) -> Result<(Dial, Step), Trouble> {
    for step in STEPS {
        match ask(step) {
            Ok(dial) if codec_ok(&dial.codec) && calm(dial.width, dial.height, f64::from(dial.fps)) => {
                crate::log_info!(
                    "cameras: pop-up stream for camera {camera}: the low stream {} {}x{} at {} a second, {}",
                    dial.codec,
                    dial.width,
                    dial.height,
                    dial.fps,
                    step.words()
                );
                return Ok((dial, step));
            }
            Ok(dial) => crate::log_info!(
                "cameras: pop-up stream for camera {camera}: {} {}x{} at {} a second is not one the panel keeps its rate with ({} asked for)",
                dial.codec,
                dial.width,
                dial.height,
                dial.fps,
                step.words()
            ),
            Err(Trouble::Bridge { status: 501, .. }) => {
                crate::log_info!("cameras: pop-up stream for camera {camera}: no stream {}", step.words());
            }
            Err(other) => return Err(other),
        }
    }
    crate::log_info!("cameras: pop-up stream for camera {camera}: no stream the panel keeps its rate with; the pop-up is drawn as a texture or not at all");
    Err(nothing_calm())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrypted::bridge::Stream;

    fn stream(quality: &str, codec: &str, w: u32, h: u32, fps: f64) -> Stream {
        Stream { quality: quality.into(), codec: codec.into(), width: w, height: h, fps, prebuffered: false }
    }

    fn camera(streams: Vec<Stream>) -> Camera {
        Camera {
            id: "240".into(),
            name: "Drive".into(),
            room: None,
            kind: "camera".into(),
            online: true,
            features: vec!["live".into()],
            earliest: None,
            native: streams.iter().find(|s| s.quality == "native").cloned(),
            streams,
        }
    }

    fn dial(codec: &str, w: u32, h: u32, fps: u32) -> Dial {
        Dial { rtsp: "rtsp://x:1/p".into(), codec: codec.into(), width: w, height: h, fps, started_at: 0, session: String::new(), rate: 1, prebuffered: true }
    }

    #[test]
    fn what_the_set_measured_is_calm_and_what_took_the_panel_to_100_hz_is_not() {
        assert!(calm(640, 480, 10.0), "camera 128: 640x480 at 10 kept 120 Hz");
        assert!(calm(1280, 720, 25.0), "camera 105: 1280x720 declared 25 kept 120 Hz");
        assert!(calm(1920, 1080, 20.0), "streams declared at 20 kept 120 Hz");
        assert!(calm(1280, 720, 30.0));
        assert!(!calm(1920, 1080, 25.0), "camera 240: 1080p at 25 took the panel to 100 Hz");
        assert!(!calm(1280, 720, 50.0), "a faster stream is not small");
        assert!(!calm(3840, 2160, 15.0), "nor is one larger than the sub window");
        assert!(!calm(1920, 1080, 0.0), "a rate nobody said is not a slow one");
        assert!(!calm(0, 0, 0.0));
        assert!(calm(0, 0, 15.0), "a rate the bridge said, with no size, is slow enough");
    }

    #[test]
    fn a_camera_whose_only_stream_is_1080p_at_25_has_no_calm_stream() {
        assert!(!window_ok(&camera(vec![stream("native", "h264", 1920, 1080, 25.0)])), "camera 240 as measured");
        assert!(!window_ok(&camera(vec![])), "no description: a texture, which cannot touch the panel");
    }

    #[test]
    fn a_low_or_slow_stream_of_any_quality_makes_a_window_calm() {
        let native = stream("native", "h264", 1920, 1080, 25.0);
        assert!(window_ok(&camera(vec![native.clone(), stream("light", "h264", 640, 360, 15.0)])), "a sub-stream");
        assert!(window_ok(&camera(vec![native.clone(), stream("balanced", "h264", 1280, 720, 25.0)])), "the remote destination");
        assert!(window_ok(&camera(vec![stream("native", "h264", 1920, 1080, 20.0)])), "a slow main stream");
        assert!(!window_ok(&camera(vec![native.clone(), stream("light", "vp9", 640, 360, 15.0)])), "a codec the window does not play");
        assert!(window_ok(&camera(vec![stream("light", "h265", 640, 360, 15.0)])));
    }

    #[test]
    fn the_small_step_comes_first_and_its_stream_is_taken() {
        let mut asked = Vec::new();
        let (d, step) = ask_window("1", |s| {
            asked.push(s);
            Ok(dial("h264", 640, 360, 15))
        })
        .unwrap();
        assert_eq!((step, d.width, asked), (Step::Small, 640, vec![Step::Small]));
    }

    #[test]
    fn a_step_with_no_stream_falls_to_the_next_and_a_camera_with_neither_is_refused() {
        let mut asked = Vec::new();
        let (d, step) = ask_window("1", |s| {
            asked.push(s);
            if s == Step::Small { Err(Trouble::Bridge { status: 501, code: "not_supported".into() }) } else { Ok(dial("h264", 1920, 1080, 20)) }
        })
        .unwrap();
        assert_eq!((step, d.fps, asked), (Step::Slow, 20, vec![Step::Small, Step::Slow]));

        let mut asked = 0;
        let refused = ask_window("1", |_| {
            asked += 1;
            Err(Trouble::Bridge { status: 501, code: "not_supported".into() })
        })
        .unwrap_err();
        assert_eq!((asked, refused.code()), (2, NO_CALM.to_owned()));
    }

    #[test]
    fn a_bridge_that_ignores_the_limits_is_not_believed() {
        // a stream of 1080p at 25 comes back for both steps: neither is calm, so neither plays
        let refused = ask_window("1", |_| Ok(dial("h264", 1920, 1080, 25))).unwrap_err();
        assert_eq!(refused.code(), NO_CALM);
    }

    #[test]
    fn any_other_refusal_ends_it_at_once() {
        let mut asked = 0;
        let refused = ask_window("1", |_| {
            asked += 1;
            Err(Trouble::Bridge { status: 409, code: "offline".into() })
        })
        .unwrap_err();
        assert_eq!((asked, refused.code()), (1, "offline".to_owned()));
    }

    #[test]
    fn the_limits_the_bridge_is_told_are_the_calm_ones() {
        assert_eq!(Step::Small.accept(), json!({"codecs": ["h264", "h265"], "max_width": 1280, "max_height": 720, "max_fps": 30}));
        assert_eq!(Step::Slow.accept(), json!({"codecs": ["h264", "h265"], "max_width": 1920, "max_height": 1088, "max_fps": 20}));
    }
}
