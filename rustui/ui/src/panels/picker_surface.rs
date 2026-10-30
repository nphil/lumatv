//! The input picker drawn (`docs/mocks/08-input-picker`): a glass sheet along the bottom with one tile for Live TV and one for each HDMI
//! port the owner has not hidden. A tile has the port, a device icon, the name the device gave itself and a state line; the input the
//! sound goes out of has a mark, and the sound output is named on the right of the head. The sheet rises from the bottom edge; the tile
//! in front is marked, the one the focus is on lifts on the flowing focus.
//!
//! The model decides which tile has the focus; this follows it. The tiles are made again when the inputs say something else (a device
//! connected, a name); the focus alone moves in place.

use super::life::Phase;
use super::picker::View;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, WordLine, Words};
use crate::host::inputs::Tile;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Spot, edge as wash_edge, hover_wash, selected_wash};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, Props, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(201);
const REGION: Region = Region(2100);
const BASE: u32 = 0x2100_0000;
const SHEET_X: f32 = 64.0;
const SHEET_W: f32 = 1792.0;
/// The mock's sheet and tile, as tall as that unless the words need more.
const SHEET_MIN_H: f32 = 404.0;
const TILE_MIN_H: f32 = 264.0;
const MARK: f32 = 26.0;

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Tile(usize),
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        match self {
            Target::Tile(i) => Id(BASE + i as u32),
            Target::Backdrop => Id(BASE + 0xFF00),
        }
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            0xFF00 => Some(Target::Backdrop),
            i if i < 0x100 => Some(Target::Tile(i as usize)),
            _ => None,
        }
    }
}

/// One tile laid out: its frame on the screen and where its parts are inside it.
#[derive(Clone, Debug, PartialEq)]
struct TileLay {
    frame: Frame,
    port: Vec<String>,
    port_at: (f32, f32, f32),
    mark: Option<(f32, f32)>,
    icon_side: u32,
    icon_at: (f32, f32),
    name: Vec<String>,
    name_at: (f32, f32, f32),
    dot: f32,
    dot_at: (f32, f32),
    state: Vec<String>,
    state_at: (f32, f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    sheet: Frame,
    title: Frame,
    sound: Option<(f32, String, f32)>,
    tiles: Vec<TileLay>,
}

fn layout(kit: &mut Kit, v: &View) -> Layout {
    let (s1, s2, s3, s4, s5, s6, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(6), kit.space(7));
    let ink = kit.ui.look.ink();
    let ink2 = kit.ui.look.ink2();
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let (label_line, caption_line, title_line) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption), kit.line(TypeRole::Title));
    let n = v.tiles.len().max(5) as f32;
    let tile_w = ((SHEET_W - 2.0 * s7 - (n - 1.0) * s4) / n).floor();
    let head_y = s5;
    let row_y = head_y + title_line + s5;
    let mut tiles = Vec::new();
    let mut tallest = 0.0f32;
    for t in &v.tiles {
        let sound = t.sound;
        let port_w = tile_w - 2.0 * s5 - if sound { MARK + s2 } else { 0.0 };
        let port = wrap::lines(kit, caption_style, &t.port, port_w, 2);
        let port_h = port.len() as f32 * caption_line;
        let icon_side = if port.len() > 1 { 44 } else { 56 };
        let icon_y = s4 + 48.0f32.max(port_h + s1);
        let name = wrap::lines(kit, label_style, &t.name, tile_w - 2.0 * s5, 12);
        let dot = s2;
        let state = wrap::lines(kit, caption_style, &t.state, tile_w - 2.0 * s5 - dot - s2, 12);
        let name_h = name.len() as f32 * label_line;
        let state_h = state.len() as f32 * caption_line;
        let row_h = dot.max(state_h);
        let words_h = name_h + row_h;
        let need = s4 + 48.0f32.max(port_h + s1) + f32::from(icon_side as u16) + s3 + words_h + s4;
        let h = TILE_MIN_H.max(need.ceil());
        tallest = tallest.max(h);
        let words_y = h - words_h - s4;
        tiles.push(TileLay {
            frame: Frame::new(0.0, 0.0, tile_w, h),
            port,
            port_at: (s5, s4, port_w),
            mark: sound.then_some((tile_w - MARK - s4, s4)),
            icon_side,
            icon_at: (s5, icon_y),
            name,
            name_at: (s5, words_y, tile_w - 2.0 * s5),
            dot,
            dot_at: (s5, words_y + name_h + ((row_h - dot) / 2.0).round()),
            state,
            state_at: (s5 + dot + s2, words_y + name_h, tile_w - 2.0 * s5 - dot - s2),
        });
    }
    let sheet_h = SHEET_MIN_H.max(row_y + tallest + s6);
    let sheet = Frame::new(SHEET_X, 1080.0 - 64.0 - sheet_h, SHEET_W, sheet_h);
    for (i, t) in tiles.iter_mut().enumerate() {
        t.frame = Frame::new(sheet.x + s7 + i as f32 * (tile_w + s4), sheet.y + row_y, tile_w, t.frame.h);
    }
    let title_w = width_of(kit, role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), "Inputs").ceil();
    let title = Frame::new(sheet.x + s7, sheet.y + head_y, title_w, title_line);
    let sound = (!v.sound.is_empty()).then(|| {
        let w = width_of(kit, role_style(kit, TypeRole::Label, None, ink2, Align::Left), &v.sound).ceil();
        (w, v.sound.clone(), MARK + s3 + w)
    });
    Layout { sheet, title, sound, tiles }
}

fn other(e: std::io::Error) -> std::io::Error {
    e
}

struct TileParts {
    id: Id,
    plate: Plate,
    _base: ShapeLayer,
    _port: Words,
    _mark: Option<ImageLayer>,
    _icon: Option<ImageLayer>,
    _name: Words,
    _dot: ShapeLayer,
    _state: Words,
    lay: TileLay,
    tile: Tile,
}

struct Built {
    sig: String,
    pane: Pane,
    _title: Words,
    sound: Option<(ImageLayer, Words, Frame)>,
    tiles: Vec<TileParts>,
    over_video: bool,
    lay: Layout,
}

pub struct PickerSurface {
    stage: Stage,
    built: Option<Built>,
}

fn tile_signature(t: &Tile) -> String {
    format!("{}|{}|{}|{}|{}|{}|{}|{}|{}", t.app_id, t.port, t.icon, t.name, t.state, t.on, t.connected, t.front, t.sound)
}

impl PickerSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    /// The sheet's place while it is built: what the mute mark keeps clear of.
    pub fn sheet(&self) -> Option<Frame> {
        self.built.as_ref().map(|b| b.lay.sheet)
    }

    fn signature(kit: &Kit, v: &View, lay: &Layout) -> String {
        format!("{}|{}|{}|{}|{:?}", kit.generation(), kit.text_scale, v.sound, v.tiles.iter().map(tile_signature).collect::<Vec<_>>().join("/"), lay.sheet)
    }

    fn fill(kit: &Kit, front: bool) -> Shape {
        let radius = kit.tokens.radius(Radius::Card);
        Shape { corner_radius: radius, fill: Fill::Solid(if front { selected_wash(kit) } else { hover_wash(kit) }), border_width: 1.0, border: wash_edge(kit) }
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, v: &View, lay: Layout, sig: String) -> std::io::Result<Built> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2, positive, ink3) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.positive(), kit.ui.look.ink3());
        let radius = kit.tokens.radius(Radius::Card);
        scene.adopting_view(root, || -> std::io::Result<Built> {
            let pane = Pane::new(kit, lay.sheet, z + 1, PaneKind::Sheet, v.over_video);
            let title = Words::new(kit, lay.title.x, lay.title.y, lay.title.w + 2.0, z + 4, &[WordLine::new("Inputs", TypeRole::Title, ink).weight(Weight::SemiBold)])?;
            let sound = match &lay.sound {
                Some((w, text, total)) => {
                    let x = lay.sheet.x + lay.sheet.w - kit.space(7) - total;
                    let cy = lay.title.y + lay.title.h / 2.0;
                    let icon_layer = icon::layer(kit, "volume-2", MARK as u32, ink2, z + 4, Props::at(x, (cy - MARK / 2.0).round()))?;
                    let line = kit.line(TypeRole::Label);
                    let words = Words::new(kit, x + MARK + kit.space(3), (cy - line / 2.0).round(), w + 2.0, z + 4, &[WordLine::new(text, TypeRole::Label, ink2)])?;
                    icon_layer.map(|i| (i, words, Frame::new(x, cy - MARK / 2.0, *total, MARK)))
                }
                None => None,
            };
            let mut tiles = Vec::new();
            for (i, (t, tl)) in v.tiles.iter().zip(lay.tiles.iter()).enumerate() {
                let id = Target::Tile(i).id();
                let f = tl.frame;
                let plate_parts = Plate::new(kit, engine, root, Spot { id, screen: f, local: f, region: REGION, scope: SCOPE, z: z + 3 }, Lift::Card, radius)?;
                let base = plate::shape(&kit.ui, f, z + 2, Self::fill(kit, t.front));
                let port_lines: Vec<WordLine> = tl.port.iter().map(|s| WordLine::new(s, TypeRole::Caption, ink2)).collect();
                let port = Words::new(kit, f.x + tl.port_at.0, f.y + tl.port_at.1, tl.port_at.2, z + 4, &port_lines)?;
                let mark = match tl.mark {
                    Some((x, y)) => icon::layer(kit, "volume-2", MARK as u32, positive, z + 4, Props::at(f.x + x, f.y + y))?,
                    None => None,
                };
                let icon_layer = icon::layer(kit, &t.icon, tl.icon_side, if t.on { ink } else { ink2 }, z + 4, Props::at(f.x + tl.icon_at.0, f.y + tl.icon_at.1))?;
                let name_lines: Vec<WordLine> = tl.name.iter().map(|s| WordLine::new(s, TypeRole::Label, ink)).collect();
                let name = Words::new(kit, f.x + tl.name_at.0, f.y + tl.name_at.1, tl.name_at.2, z + 4, &name_lines)?;
                let dot = plate::shape(&kit.ui, Frame::new(f.x + tl.dot_at.0, f.y + tl.dot_at.1, tl.dot, tl.dot), z + 4, plate::solid(if t.connected { positive } else { ink3 }, tl.dot / 2.0));
                let state_lines: Vec<WordLine> = tl.state.iter().map(|s| WordLine::new(s, TypeRole::Caption, ink2)).collect();
                let state = Words::new(kit, f.x + tl.state_at.0, f.y + tl.state_at.1, tl.state_at.2, z + 4, &state_lines)?;
                tiles.push(TileParts { id, plate: plate_parts, _base: base, _port: port, _mark: mark, _icon: icon_layer, _name: name, _dot: dot, _state: state, lay: tl.clone(), tile: t.clone() });
            }
            Ok(Built { sig, pane, _title: title, sound, tiles, over_video: v.over_video, lay: lay.clone() })
        })
        .map_err(other)
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for t in b.tiles {
                t.plate.destroy(engine);
            }
        }
    }

    /// Fits the picker to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let lay = layout(kit, v);
        let arrive = Arrive::Bottom(lay.sheet.h + 64.0);
        if self.stage.needs_root(arrive) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, arrive, true)?;
        let sig = Self::signature(kit, v, &lay);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), v, lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the input picker could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, phase) {
            if let Some(b) = self.built.as_mut()
                && b.over_video != v.over_video
            {
                b.pane.restyle(kit, v.over_video);
                b.over_video = v.over_video;
            }
            self.stage.focus(engine, (!v.tiles.is_empty()).then(|| Target::Tile(v.index).id()), now_ns);
        }
        Ok(true)
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it, and a click is the target it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let radius = kit.tokens.radius(Radius::Card);
        let id = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(Target::from_id).filter(|t| matches!(t, Target::Tile(_))).map(|_| radius),
            Touch::Press { .. } => None,
        })?;
        Target::from_id(id)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    /// How many layers the picker holds.
    pub fn layers(&self) -> usize {
        let Some(b) = &self.built else { return 0 };
        2 + 1 + b.sound.as_ref().map_or(0, |_| 2) + b.tiles.iter().map(|t| 1 + 1 + t._port.layers() + usize::from(t._mark.is_some()) + usize::from(t._icon.is_some()) + t._name.layers() + 1 + t._state.layers()).sum::<usize>()
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let tiles: Vec<Value> = b.tiles.iter().map(|t| json!({"frame": f(engine.focus.frame(t.id).unwrap_or(t.lay.frame)), "name": t.tile.name, "port": t.tile.port, "front": t.tile.front, "sound": t.tile.sound})).collect();
        json!({"sheet": f(b.lay.sheet), "tiles": tiles, "focus": engine.focus.current().and_then(Target::from_id).and_then(|t| if let Target::Tile(i) = t { Some(i) } else { None }), "layers": self.layers(), "scope": SCOPE.0})
    }

    /// Describes what is on the screen to a lint: the sheet, each tile over it, and the words on them.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = lint.panel("input sheet", b.lay.sheet, kit.ui.look.colour(PaletteRole::Sheet), false);
        b._title.feed_lint(kit, lint, "inputs title", Some(sheet), (0.0, 0.0));
        if let Some((_, words, _)) = &b.sound {
            words.feed_lint(kit, lint, "inputs sound", Some(sheet), (0.0, 0.0));
        }
        for (i, t) in b.tiles.iter().enumerate() {
            let paint = if t.tile.front { selected_wash(kit) } else { hover_wash(kit) };
            let card = lint.panel_on(&format!("tile {i}"), t.lay.frame, paint, false, sheet);
            t._port.feed_lint(kit, lint, &format!("tile {i} port"), Some(card), (0.0, 0.0));
            t._name.feed_lint(kit, lint, &format!("tile {i} name"), Some(card), (0.0, 0.0));
            t._state.feed_lint(kit, lint, &format!("tile {i} state"), Some(card), (0.0, 0.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::life::Phase;
    use crate::panels::env::Env;
    use crate::panels::picker::Picker;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: PickerSurface,
        p: Picker,
        env: Env,
        t: u64,
    }

    fn tv(devices: Value) -> Env {
        // every port is shown, the third with nothing on it
        let mut env = Env::new(json!({"inputs": {"hdmi1": "always", "hdmi2": "always", "hdmi3": "always", "hdmi4": "always"}}));
        env.inputs.parse(&json!({"devices": devices}));
        env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi2", "home": false}) }, 0);
        env
    }

    fn devices(n: usize) -> Value {
        let names = ["Apple TV 4K", "Denon AVR-S760H", "PlayStation 5 with a rather long name that wraps", "Nintendo Switch", "Blu-ray player", "Fire TV", "Shield", "Xbox"];
        Value::Array((0..n).map(|i| json!({"appId": format!("com.webos.app.hdmi{}", i + 1), "label": names[i % names.len()], "connected": i != 2})).collect())
    }

    impl Up {
        fn new(mode: Mode, scale: f32, n: usize) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut env = tv(devices(n));
            env.sio.write("sound.soundOutput", &json!("external_arc"), None);
            let mut p = Picker::new();
            p.life.show(0);
            p.open(&mut env, 0);
            let mut me = Self { _h: h, kit, engine, s: PickerSurface::new(crate::kit::z::PANELS), p, env, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.p.view(&self.env);
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.p.life.phase(), self.t * 1_000_000).expect("the picker is drawn");
        }

        fn key(&mut self, k: K) {
            self.p.key(k, &self.env);
            self.t += 40;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }
    }

    #[test]
    fn the_sheet_rests_64_from_the_bottom_and_holds_a_tile_per_input_in_one_row() {
        let u = Up::new(Mode::Dark, 1.0, 4);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["sheet"]["x"], 64.0);
        assert_eq!(g["sheet"]["w"], 1792.0);
        let sheet = &g["sheet"];
        assert_eq!(sheet["y"].as_f64().unwrap() + sheet["h"].as_f64().unwrap(), 1080.0 - 64.0);
        assert!(sheet["h"].as_f64().unwrap() >= 404.0);
        let tiles = g["tiles"].as_array().expect("tiles");
        assert_eq!(tiles.len(), 5, "Live TV and the four ports");
        let w0 = tiles[0]["frame"]["w"].as_f64().unwrap();
        assert!(tiles.iter().all(|t| (t["frame"]["w"].as_f64().unwrap() - w0).abs() < 0.5 && t["frame"]["h"].as_f64().unwrap() >= 264.0));
        assert_eq!(w0, ((1792.0f64 - 2.0 * 32.0 - 4.0 * 16.0) / 5.0).floor(), "five columns even when there are fewer tiles: {w0}");
        assert_eq!(g["focus"], 2, "on the input in front");
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
    }

    #[test]
    fn the_focus_follows_the_choice_with_a_lift_and_the_first_one_is_lifted_at_once() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        assert!(!u.engine.flow.settling_at(u.t * 1_000_000));
        u.key(K::Right);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 3);
        assert!(u.engine.flow.settling_at(u.t * 1_000_000));
        u.key(K::Left);
        u.key(K::Left);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 1);
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_with_few_tiles_and_many() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for n in [2, 3, 4] {
                    let mut u = Up::new(mode, scale, n);
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} {n} ports: {f:#?}");
                }
            }
        }
    }

    #[test]
    fn a_change_in_the_inputs_makes_the_tiles_again_and_a_key_changes_nothing_in_the_scene() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let layers = u.kit.room().layers;
        u.key(K::Right);
        u.key(K::Left);
        assert_eq!(u.kit.room().layers, layers, "the focus is the flowing lift: no layer is made");
        let before = u.s.geometry(&u.engine)["tiles"][2]["name"].clone();
        u.env.inputs.parse(&json!({"devices": devices(1)}));
        u.p.sync(&u.env);
        u.apply();
        assert_ne!(u.s.geometry(&u.engine)["tiles"][2]["name"], before, "the tile that lost its device says so");
        assert_eq!(u.s.geometry(&u.engine)["tiles"].as_array().map(Vec::len), Some(2), "Live TV and the one port left");
    }

    #[test]
    fn a_click_on_a_tile_names_it_and_the_dim_is_a_target_that_closes() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let f = u.engine.focus.frame(Target::Tile(1).id()).expect("a tile");
        let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
        let t = |n: u64| (3000 + n) * 1_000_000;
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(f.x + 20.0, f.y + 20.0, 0)), None);
        assert!(u.s.stage.hover.is_shown(), "a cue under the pointer");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(f.x + 20.0, f.y + 20.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(f.x + 20.0, f.y + 20.0, 0)), Some(Target::Tile(1)));
        // the dim, above the sheet
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(600.0, 100.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(600.0, 100.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(50), at(600.0, 100.0, 0)), Some(Target::Backdrop));
        u.s.on_key(&mut u.engine);
        assert!(!u.s.stage.hover.is_shown());
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Tile(0), Target::Tile(7), Target::Backdrop] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
        assert_eq!(Target::from_id(Id(1)), None);
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let with = u.kit.room().layers;
        u.p.close(&mut u.env, 2000);
        u.p.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.p.life.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.p.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 20 && !u.s.is_up());
    }

    fn centre(f: Frame) -> (f32, f32) {
        (f.x + f.w / 2.0, f.y + f.h / 2.0)
    }

    /// The tiles whose lit plate is up by more than half, once the springs have settled (the scene's own clock runs them).
    fn lit(u: &Up) -> Vec<usize> {
        let at = u.kit.ui.scene.now_ns() + 3_000_000_000;
        (0..8).filter(|i| u.engine.flow.lift_at(Target::Tile(*i).id(), at) > 0.5).collect()
    }

    #[test]
    fn the_lifted_plate_is_concentric_with_its_tile_on_every_tile_and_at_both_sizes() {
        for scale in [1.0, 2.0] {
            let u = Up::new(Mode::Dark, scale, 3);
            for i in 0..4 {
                let id = Target::Tile(i).id();
                let tile = u.engine.focus.frame(id).expect("a tile");
                let (plate, _) = u.engine.flow.lifted_plate(id, u.kit.ui.scene.now_ns() + 5_000_000_000).expect("a plate");
                let (a, b) = (centre(tile), centre(plate));
                assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01, "tile {i} at {scale}: plate {plate:?} is off its tile {tile:?}");
                assert!(plate.w >= tile.w && plate.h >= tile.h, "the plate is the tile grown, never smaller");
            }
        }
    }

    #[test]
    fn the_ring_is_on_the_tile_the_keys_chose_and_on_it_alone_also_after_the_picker_is_opened_again() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        assert_eq!(lit(&u), vec![2], "opened on the input in front");
        u.key(K::Left);
        assert_eq!(lit(&u), vec![1]);
        u.key(K::Left);
        u.key(K::Left);
        assert_eq!(lit(&u), vec![0], "on Live TV");
        // closed and gone, then opened again with no key: the ring is on the tile in front and nowhere else
        u.p.close(&mut u.env, 2000);
        u.p.life.hide(2000);
        u.apply();
        u.p.life.tick(2200);
        u.apply();
        assert_eq!(u.p.life.phase(), Phase::Parked, "its layers are kept for a while");
        u.t = 2500;
        u.p.life.show(u.t);
        u.p.open(&mut u.env, u.t);
        u.apply();
        assert_eq!(lit(&u), vec![2], "no ghost ring on the other tiles");
    }
}
