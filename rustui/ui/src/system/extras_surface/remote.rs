//! The on-screen remote drawn (`panels/ScreenRemote.qml`): a sheet along the foot of the screen, 64 from the edges, with the keys the micom service
//! presses for the person who has no remote in hand (the transport keys as pills, the number pad with Info and a key that closes the sheet, the four
//! colours). It has no OK, no Back and no arrow ([`crate::system::extras::REMOTE_KEYS`]): nothing here can answer a prompt that belongs to the physical
//! remote. The keys the sheet itself answers to are glyphs in its head (the D-pad moves, OK presses, Back closes), never sentences.
//!
//! The model decides which key has the focus; this follows it with the flowing lift. A key's glyph stays where it is while the key lifts. At a large
//! text size the number pad's keys grow with the words on them and the sheet grows upward; the order and the neighbours are the model's.

use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Spot, edge, hover_wash};
use crate::kit::{Engine, Kit, avatar};
use crate::panels::life::Phase;
use crate::panels::stage::{Stage, Touch};
use crate::panels::ui::{Arrive, hints_width};
use crate::system::extras::{Face, REMOTE_KEYS};
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(211);
const REGION: Region = Region(3100);
const BASE: u32 = 0x3200_0000;
const SHEET: Frame = Frame::new(64.0, 656.0, 1792.0, 360.0);
const FACE: u32 = 36;
const NUMBER_H: f32 = 54.0;
const PILL_H: f32 = 88.0;

fn id_of(i: usize) -> Id {
    Id(BASE + i as u32)
}

/// The key a click or hover landed on.
pub fn key_of(id: Id) -> Option<usize> {
    id.0.checked_sub(BASE).filter(|i| (*i as usize) < REMOTE_KEYS.len()).map(|i| i as usize)
}

#[derive(Clone, Debug, PartialEq)]
struct Lay {
    sheet: Frame,
    title: Frame,
    hints_at: (f32, f32, f32),
    hints: Vec<Hint>,
    keys: Vec<Frame>,
}

fn head_hints() -> Vec<Hint> {
    vec![
        Hint { keys: vec![(Key::Up, false), (Key::Down, false), (Key::Left, false), (Key::Right, false)], action: Some(Action::Move), label: None },
        Hint::key(Key::Ok, Action::Confirm),
        Hint::key(Key::Back, Action::Close),
    ]
}

/// The keys where they are at this text size: the model's grid (`remote_box`) with the number pad's keys as wide and as tall as their words need,
/// the rows of the pad further apart, the sheet taller by what the pad needs over its 54 and the head by what its title needs.
fn layout(kit: &mut Kit) -> Lay {
    let (s3, s4, s6, s7) = (kit.space(3), kit.space(4), kit.space(6), kit.space(7));
    let ink = kit.ui.look.ink();
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Centre);
    let title_h = kit.line(TypeRole::Title);
    let head_h = (96.0f32).max(s4 + title_h + s4).ceil();
    let number_h = NUMBER_H.max(kit.line(TypeRole::Label) + s3).ceil();
    let number_w = 112.0f32.max(width_of(kit, label_style, "Info") + 2.0 * s4).ceil();
    let pitch = number_h + 8.0;
    let body_h = (40.0 + PILL_H).max(4.0 * pitch - 8.0);
    let h = head_h + body_h + s6;
    let sheet = Frame::new(SHEET.x, 1080.0 - 64.0 - h, SHEET.w, h);
    let (bx, by) = (sheet.x + s7, sheet.y + head_h);
    // the pad is wider by what its keys grew by, the gaps between the three groups give way
    let pad_w = 3.0 * number_w + 2.0 * 16.0;
    let g0_w = 5.0 * 112.0 + 4.0 * 16.0;
    let g2_w = 4.0 * 120.0 + 3.0 * 10.0;
    let inner = sheet.w - 2.0 * s7;
    let gap = ((inner - g0_w - pad_w - g2_w) / 2.0).floor().max(16.0);
    let pad_x = g0_w + gap;
    let g2_x = pad_x + pad_w + gap;
    let keys = (0..REMOTE_KEYS.len())
        .map(|i| match REMOTE_KEYS[i].group {
            0 => Frame::new(bx + i as f32 * 128.0, by + 40.0, 112.0, PILL_H),
            1 => {
                let slot = i - 5;
                Frame::new(bx + pad_x + (slot % 3) as f32 * (number_w + 16.0), by + (slot / 3) as f32 * pitch, number_w, number_h)
            }
            _ => Frame::new(bx + g2_x + (i - 17) as f32 * 130.0, by + 40.0, 120.0, PILL_H),
        })
        .collect();
    let title_w = width_of(kit, role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), "Remote").ceil();
    let title = Frame::new(sheet.x + s7, sheet.y + ((head_h - title_h) / 2.0).round().min(s4 + 4.0), title_w, title_h);
    // the head's keys at the right, as many as there is room for
    let mut hints = head_hints();
    while hints.len() > 1 && hints_width(kit, &hints, TypeRole::Label, s7) > sheet.w - 2.0 * s7 - title_w - s7 {
        hints.remove(0);
    }
    let hints_w = hints_width(kit, &hints, TypeRole::Label, s7);
    let hints_at = (sheet.right() - s7 - hints_w, title.y, title_h);
    Lay { sheet, title, hints_at, hints, keys }
}

struct KeyParts {
    plate: Plate,
    _base: ShapeLayer,
    /// What is drawn on the key (a glyph, its word, a colour's bar): held so it stays on the screen as long as the key does.
    _face: Vec<Box<dyn std::any::Any>>,
}

struct Built {
    sig: String,
    lay: Lay,
    _pane: Pane,
    _title: Text,
    hint_rows: Vec<HintRow>,
    keys: Vec<KeyParts>,
    focus: usize,
}

pub struct RemoteSurface {
    stage: Stage,
    built: Option<Built>,
}

/// The same icon turned about its vertical: the rewind key is the fast-forward key looking the other way (the icon set has no rewind).
fn flipped(kit: &mut Kit, name: &str, side: u32, ink: Rgba, z: i32, at: Props) -> std::io::Result<Option<ImageLayer>> {
    let Some(px) = kit.icons.draw(name, side, ink) else { return Ok(None) };
    let mut layer = kit.ui.scene.image(side, side, z, at)?;
    let row = (side * 4) as usize;
    let out = layer.pixels_mut();
    for y in 0..side as usize {
        for x in 0..side as usize {
            let (from, to) = ((y * row) + x * 4, (y * row) + (side as usize - 1 - x) * 4);
            out[to..to + 4].copy_from_slice(&px[from..from + 4]);
        }
    }
    layer.damage(None);
    Ok(Some(layer))
}

impl RemoteSurface {
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn build(kit: &mut Kit, engine: &mut Engine, group: &ui::group::Group, z: i32, lay: Lay, focus: usize, sig: String) -> std::io::Result<Built> {
        let scene = kit.ui.scene.clone();
        let ink = kit.ui.look.ink();
        let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Centre);
        let control = kit.tokens.radius(Radius::Control);
        scene.adopting_view(group, || -> std::io::Result<Built> {
            let pane = Pane::new(kit, lay.sheet, z + 1, PaneKind::Sheet, false);
            let title = Text::new(kit, lay.title, z + 2, title_style, "Remote")?;
            let mut hint_rows = Vec::new();
            let mut x = lay.hints_at.0;
            for h in &lay.hints {
                let row = HintRow::new(kit, h, TypeRole::Label, RowAt { x, y: lay.hints_at.1, h: lay.hints_at.2, z: z + 2 }, ink)?;
                x += row.width() + kit.space(7);
                hint_rows.push(row);
            }
            let mut keys = Vec::new();
            for (i, key) in REMOTE_KEYS.iter().enumerate() {
                let f = lay.keys[i];
                let radius = if key.group == 1 { control } else { f.h / 2.0 };
                let parts = Plate::new(kit, engine, group, Spot { id: id_of(i), screen: f, local: f, region: REGION, scope: SCOPE, z: z + 3 }, Lift::Button, radius)?;
                let base = plate::shape(&kit.ui, f, z + 2, Shape { corner_radius: radius, fill: Fill::Solid(hover_wash(kit)), border_width: 1.0, border: edge(kit) });
                let mut face: Vec<Box<dyn std::any::Any>> = Vec::new();
                let centre = (f.x + f.w / 2.0, f.y + f.h / 2.0);
                let glyph_at = Props::at((centre.0 - FACE as f32 / 2.0).round(), (centre.1 - FACE as f32 / 2.0).round());
                match key.face {
                    Face::Icon(name) => {
                        if let Some(l) = icon::layer(kit, name, FACE, ink, z + 4, glyph_at)? {
                            face.push(Box::new(l));
                        }
                    }
                    Face::Rewind => {
                        if let Some(l) = flipped(kit, "fast-forward", FACE, ink, z + 4, glyph_at)? {
                            face.push(Box::new(l));
                        }
                    }
                    Face::Text(t) => {
                        let line = kit.line(TypeRole::Label);
                        face.push(Box::new(Text::new(kit, Frame::new(f.x, (centre.1 - line / 2.0).round(), f.w, line), z + 4, label_style, t)?));
                    }
                    Face::Colour(hex) => {
                        let c = avatar::parse(hex, ink);
                        face.push(Box::new(plate::shape(&kit.ui, Frame::new((centre.0 - 28.0).round(), (centre.1 - 8.0).round(), 56.0, 16.0), z + 4, plate::solid(c, 8.0))));
                    }
                }
                keys.push(KeyParts { plate: parts, _base: base, _face: face });
            }
            Ok(Built { sig, lay: lay.clone(), _pane: pane, _title: title, hint_rows, keys, focus })
        })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for k in b.keys {
                k.plate.destroy(engine);
            }
        }
    }

    /// Fits the sheet to the model: made when the remote opens (or the look or the text size change), the focus follows the model's key, dropped at
    /// once when it has closed. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: Option<usize>, now_ns: u64) -> std::io::Result<bool> {
        let Some(focus) = view else {
            self.drop_all(engine);
            return Ok(false);
        };
        let lay = layout(kit);
        let arrive = Arrive::Bottom(lay.sheet.h + 64.0);
        if self.stage.needs_root(arrive) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, Phase::Opening, arrive, false)?;
        let sig = format!("{}|{}|{:?}", kit.generation(), kit.text_scale, lay.sheet);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), lay, focus, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the on-screen remote could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, Phase::Opening) {
            self.stage.focus(engine, Some(id_of(focus)), now_ns);
            if let Some(b) = self.built.as_mut() {
                b.focus = focus;
            }
        }
        Ok(true)
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it, and a click is the key it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<usize> {
        let built = &self.built;
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(key_of).and_then(|i| built.as_ref().and_then(|b| b.lay.keys.get(i))).map(|f| if f.h > 60.0 && f.h < 100.0 { f.h / 2.0 } else { kit.tokens.radius(Radius::Control) }),
            Touch::Press { .. } => None,
        })?;
        key_of(click)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let keys: Vec<Value> = REMOTE_KEYS.iter().zip(&b.keys).map(|(k, p)| json!({"name": k.name, "frame": f(engine.focus.frame(p.plate.id).unwrap_or(b.lay.keys[0]))})).collect();
        json!({
            "sheet": f(b.lay.sheet),
            "keys": keys,
            "hints": b.hint_rows.iter().map(|h| h.name().to_string()).collect::<Vec<_>>(),
            "focus": engine.focus.current().and_then(key_of).map(|i| REMOTE_KEYS[i].name),
            "scope": SCOPE.0,
        })
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = lint.panel("remote sheet", b.lay.sheet, kit.ui.look.colour(PaletteRole::Sheet), true);
        let ink = kit.ui.look.ink();
        let title = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        lint.word(kit, "remote title", "Remote", b.lay.title, title, Some(sheet));
        let label = role_style(kit, TypeRole::Label, None, ink, Align::Centre);
        let wash = hover_wash(kit);
        for (i, k) in REMOTE_KEYS.iter().enumerate() {
            let on = lint.panel_on(&format!("remote key {}", k.name), b.lay.keys[i], wash, false, sheet);
            if let Face::Text(t) = k.face {
                lint.word(kit, &format!("remote key {} label", k.name), t, b.lay.keys[i], label, Some(on));
            }
        }
    }
}
