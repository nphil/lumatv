//! The Cameras app as the shell holds it: the model ([`App`]) and its surface ([`CamerasSurface`]) with the glue between them and the host:
//! a still or a thumbnail that arrives as bytes is decoded into the surface's store (the model sees a picture or a refusal, never the bytes),
//! the model's view is drawn with the pictures the store has, the pointer's click is the model's target, and what the model lets go of the
//! store lets go of. No I/O: the effects the model returns are the shell's to carry out.

use super::app::{App, Config, Intent};
use super::pics;
use super::surface::CamerasSurface;
use crate::kit::lint::Lint;
use crate::kit::picture;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input, Model};
use crate::kit::{Engine, Kit};
use serde_json::{Value, json};

pub struct Cameras {
    pub app: App,
    pub surface: CamerasSurface,
}

impl Cameras {
    /// `z` is the band's base (`system_z + kit::z::CAMERAS`).
    pub fn new(z: i32, cfg: Config) -> Self {
        Self { app: App::new(cfg), surface: CamerasSurface::new(z) }
    }

    /// A topic, a helper's answer, a key or the clock for the model. A picture's bytes are decoded here.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let fx = match input {
            Input::Bytes { tag, status, bytes } if pics::owns(*tag) => {
                let answer = match (*status, self.app.picture_key(*tag)) {
                    (200, Some(key)) => match picture::decode(bytes) {
                        Ok(pic) => {
                            let (w, h) = (pic.width, pic.height);
                            self.surface.put_picture(&key, pic);
                            Input::Fetched { tag: *tag, status: 200, data: json!({"picture": {"w": w, "h": h}}) }
                        }
                        Err(e) => Input::Fetched { tag: *tag, status: 502, data: json!({"error": {"code": "decode", "message": e}}) },
                    },
                    (status, _) => Input::Fetched { tag: *tag, status, data: Value::Null },
                };
                self.app.update(&answer, now)
            }
            other => self.app.update(other, now),
        };
        let dropped = self.app.take_dropped();
        if !dropped.is_empty() {
            self.surface.forget(&dropped);
        }
        fx
    }

    /// Draws the model's view with the pictures the store has. Returns whether anything of the app is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, now_ms: u64) -> std::io::Result<bool> {
        let Self { app, surface } = self;
        let view = app.view_with(&|key| surface.has_picture(key));
        surface.apply(kit, engine, &view, now_ms)
    }

    /// A key of the remote: the pointer's cue goes, then the model's say.
    pub fn key(&mut self, name: &str, down: bool, repeat: bool, kit: &mut Kit, engine: &mut Engine, now: u64) -> Vec<Effect> {
        if down && self.app.viewer_up() {
            self.surface.on_key(kit, engine);
        }
        self.update(&Input::Key { name: name.to_string(), down, repeat }, now)
    }

    /// A sample of the pointer: a click on a control is the model's `click`.
    pub fn pointer(&mut self, kit: &mut Kit, engine: &mut Engine, now_ns: u64, now_ms: u64, sample: Sample) -> Vec<Effect> {
        match self.surface.pointer(kit, engine, now_ns, sample) {
            Some(target) => {
                let fx = self.app.click(target, now_ms);
                let dropped = self.app.take_dropped();
                if !dropped.is_empty() {
                    self.surface.forget(&dropped);
                }
                fx
            }
            None => Vec::new(),
        }
    }

    /// Time passed. Returns the model's effects and whether the surface let something go.
    pub fn tick(&mut self, now: u64) -> (Vec<Effect>, bool) {
        let fx = self.update(&Input::Tick, now);
        (fx, self.surface.tick(now))
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.app.due_at(), self.surface.due_at()].into_iter().flatten().min()
    }

    /// The layer wants every key while the viewer is up.
    pub fn wants(&self) -> bool {
        self.app.viewer_up()
    }

    pub fn is_live(&self) -> bool {
        self.surface.is_live()
    }

    /// The pop-up's OK: the full view of `camera`.
    pub fn open_full(&mut self, camera: &str, now: u64) -> Vec<Effect> {
        self.app.open_full(camera, now)
    }

    /// The pop-up's picture of `camera`, put where the full view draws its still: the view opens on a picture in its first frame, and the video replaces it.
    pub fn seed_full(&mut self, camera: &str, pic: picture::Picture) {
        self.surface.put_picture(&super::app::full_key(camera), pic);
    }

    /// The grid of every camera (the quick panel's tile).
    pub fn open_grid(&mut self, now: u64) -> Vec<Effect> {
        self.app.open_grid(now)
    }

    /// The recent events of every camera over the grid.
    pub fn open_events(&mut self, now: u64) -> Vec<Effect> {
        self.app.open_events(now)
    }

    pub fn close(&mut self, now: u64) -> Vec<Effect> {
        let fx = self.app.close(now);
        let dropped = self.app.take_dropped();
        self.surface.forget(&dropped);
        fx
    }

    /// What the app asks of the shell besides effects (the grid's `Settings` button).
    pub fn take_intents(&mut self) -> Vec<Intent> {
        self.app.take_intents()
    }

    /// The count of cameras and the viewer's state, for the report's `state` (the gate reads it) and the harness.
    pub fn state(&self) -> Value {
        let mut s = self.app.debug();
        s["layers"] = json!(self.surface.layers());
        s
    }

    pub fn geometry(&self) -> Value {
        self.surface.geometry()
    }

    /// The layout and contrast lint of what is on the screen: an object per finding.
    pub fn lint(&mut self, kit: &mut Kit) -> Value {
        let mut lint = Lint::new().with_safe(ui::geometry::Frame::new(64.0, 54.0, 1792.0, 972.0));
        self.surface.feed_lint(kit, &mut lint);
        json!(lint.findings().iter().map(|f| json!({"kind": f.kind.name(), "subject": f.subject, "detail": f.detail})).collect::<Vec<_>>())
    }

    /// The look or the text size changed: everything is made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine, text_scale: f32) {
        self.app.cfg.text_scale = text_scale;
        self.surface.restyle(engine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use serde_json::json;

    fn camera(id: &str, name: &str) -> Value {
        json!({"id": id, "name": name, "online": true, "live": true, "recordings": true, "events": true, "clips": false, "prebuffered": true})
    }

    fn rig() -> (ui::testing::Harness, Kit, Engine, Cameras) {
        let (h, kit) = testkit::kit(1.0);
        let engine = Engine::new(&kit).unwrap();
        let mut c = Cameras::new(2000, Config::default());
        c.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "ready", "cameras": [camera("273", "Front Door"), camera("274", "Back Door")]}) }, 1000);
        (h, kit, engine, c)
    }

    fn bmp(w: u32, h: u32, bgr: [u8; 3]) -> Vec<u8> {
        let stride = (w * 3).div_ceil(4) * 4;
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[2..6].copy_from_slice(&(54 + stride * h).to_le_bytes());
        b[10..14].copy_from_slice(&54u32.to_le_bytes());
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&w.to_le_bytes());
        b[22..26].copy_from_slice(&h.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&24u16.to_le_bytes());
        for _ in 0..h {
            for _ in 0..w {
                b.extend_from_slice(&bgr);
            }
            b.extend(std::iter::repeat_n(0u8, (stride - w * 3) as usize));
        }
        b
    }

    fn frame_asks(fx: &[Effect]) -> Vec<(u64, String)> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Fetch { tag, path } if path.contains("/frame?") => Some((*tag, path.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_still_that_arrives_as_bytes_is_decoded_into_the_store_and_painted_over_its_tile() {
        let (_h, mut kit, mut engine, mut c) = rig();
        let fx = c.open_grid(1000);
        let asks = frame_asks(&fx);
        assert_eq!(asks.len(), 2);
        assert!(c.apply(&mut kit, &mut engine, 1000).unwrap());
        assert!(!c.surface.has_picture("tile:273"));
        let (tag, _) = asks.iter().find(|(_, p)| p.contains("/273/")).cloned().unwrap();
        let fx = c.update(&Input::Bytes { tag, status: 200, bytes: bmp(32, 18, [10, 20, 200]) }, 1100);
        assert!(fx.iter().any(|e| matches!(e, Effect::Redraw)), "a picture is a reason to draw");
        assert!(c.surface.has_picture("tile:273"));
        c.apply(&mut kit, &mut engine, 1100).unwrap();
        assert!(c.surface.layers() > 0);
    }

    #[test]
    fn bytes_that_are_not_a_picture_count_as_a_refusal_and_a_refusal_as_the_helper_said_it() {
        let (_h, mut kit, mut engine, mut c) = rig();
        let fx = c.open_grid(1000);
        let asks = frame_asks(&fx);
        c.apply(&mut kit, &mut engine, 1000).unwrap();
        let (tag, _) = asks[0].clone();
        c.update(&Input::Bytes { tag, status: 200, bytes: b"<html>".to_vec() }, 1100);
        assert!(!c.surface.has_picture(&format!("tile:{}", "273")));
        let (tag2, _) = asks[1].clone();
        c.update(&Input::Bytes { tag: tag2, status: 404, bytes: Vec::new() }, 1100);
        assert!(!c.surface.has_picture("tile:274"));
        assert!(c.app.debug()["inFlight"].as_u64().unwrap() == 0);
    }

    #[test]
    fn closing_takes_everything_off_and_the_pictures_stay_for_the_next_first_frame() {
        let (_h, mut kit, mut engine, mut c) = rig();
        let fx = c.open_grid(1000);
        let (tag, _) = frame_asks(&fx)[0].clone();
        c.update(&Input::Bytes { tag, status: 200, bytes: bmp(32, 18, [1, 2, 3]) }, 1050);
        c.apply(&mut kit, &mut engine, 1050).unwrap();
        assert!(c.surface.is_up());
        c.close(1100);
        let before = kit.room();
        c.apply(&mut kit, &mut engine, 1100).unwrap();
        assert!(!c.surface.is_up());
        assert!(kit.room().layers >= before.layers, "{:?} -> {:?}", before, kit.room());
        assert!(c.surface.has_picture("tile:273"), "the last picture is kept: the grid paints it in its first frame");
        assert!(!engine.focus.is_live(crate::cameras::surface::ids::tile(0)));
    }

    #[test]
    fn the_second_open_paints_the_kept_picture_in_the_first_frame() {
        let (_h, mut kit, mut engine, mut c) = rig();
        let fx = c.open_grid(1000);
        let (tag, _) = frame_asks(&fx)[0].clone();
        c.update(&Input::Bytes { tag, status: 200, bytes: bmp(32, 18, [1, 2, 3]) }, 1050);
        c.close(1100);
        c.apply(&mut kit, &mut engine, 1100).unwrap();
        c.open_grid(2000);
        c.apply(&mut kit, &mut engine, 2000).unwrap();
        let g = c.geometry();
        assert_eq!(g["base"], "grid");
        assert!(g["layers"].as_u64().unwrap() > 20);
    }

    #[test]
    fn a_picture_seeded_for_the_full_view_is_the_stores_before_the_view_asks_for_any() {
        let (_h, _kit, _engine, mut c) = rig();
        assert!(!c.surface.has_picture("full:273"));
        c.seed_full("273", picture::Picture { width: 4, height: 4, rgba: vec![200; 64] });
        assert!(c.surface.has_picture("full:273"), "the view draws it in its first frame, as the grid draws the kept tiles");
    }

    #[test]
    fn a_click_on_a_tile_opens_the_camera() {
        let (_h, mut kit, mut engine, mut c) = rig();
        c.open_grid(1000);
        c.apply(&mut kit, &mut engine, 1000).unwrap();
        let rect = c.surface.geometry()["tiles"][1].clone();
        let (x, y) = (rect["x"].as_f64().unwrap() as f32 + 20.0, rect["y"].as_f64().unwrap() as f32 + 20.0);
        let s = |buttons| Sample { x, y, buttons, visible: true };
        c.pointer(&mut kit, &mut engine, 1, 1010, s(0));
        let down = c.pointer(&mut kit, &mut engine, 2, 1020, s(1));
        assert!(down.is_empty(), "hover and press act on nothing");
        let fx = c.pointer(&mut kit, &mut engine, 3, 1030, s(0));
        assert!(fx.iter().any(|e| matches!(e, Effect::Request { path, .. } if path == "/v1/cameras/live")), "the click opened the camera: {fx:?}");
        assert_eq!(c.app.debug()["screen"]["camera"], "274");
    }

    #[test]
    fn the_lint_and_the_state_are_values_the_harness_can_print() {
        let (_h, mut kit, mut engine, mut c) = rig();
        c.open_grid(1000);
        c.apply(&mut kit, &mut engine, 1000).unwrap();
        assert_eq!(c.lint(&mut kit), json!([]));
        assert_eq!(c.state()["screen"]["mode"], "grid");
        assert!(c.wants());
    }
}
