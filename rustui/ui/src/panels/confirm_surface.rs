//! The confirm dialog drawn (`docs/mocks/06-confirm-dialog`, `panels/Confirm.qml`): the page behind is hidden, and a solid card 760 wide in
//! the middle holds the app's icon (or a glyph on the accent), the question, what happens and one button per answer. The safe answer is the
//! primary button and has the focus first; a danger answer is drawn in the danger colour. Nothing but an answer closes it: the dim is not a
//! target.
//!
//! The model decides which answer has the focus; this follows it. The card is made again when the question says something else.

use super::confirm::{Spec, View};
use super::life::Phase;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, WordLine, Words, picture_layer};
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::text::role_style;
use crate::kit::widgets::{edge, tile};
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

pub const SCOPE: Scope = Scope(204);
const REGION: Region = Region(2400);
const BASE: u32 = 0x2400_0000;
const CARD_W: f32 = 760.0;
const ICON: f32 = 56.0;

/// What the pointer can land on: an answer (the dim is no target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Answer(usize),
}

impl Target {
    pub fn id(self) -> Id {
        match self {
            Target::Answer(i) => Id(BASE + i as u32),
        }
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            i if i < 0x100 => Some(Target::Answer(i as usize)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    card: Frame,
    icon: Frame,
    title: Vec<String>,
    title_at: (f32, f32, f32),
    body: Vec<String>,
    body_at: (f32, f32, f32),
    /// The answers' buttons on the screen.
    buttons: Vec<Frame>,
}

fn layout(kit: &mut Kit, spec: &Spec) -> Layout {
    let (s4, s6) = (kit.space(4), kit.space(6));
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
    let inner = CARD_W - 2.0 * s6;
    let title_w = inner - ICON - s4;
    let title = wrap::lines(kit, title_style, &spec.title, title_w, 3);
    let title_h = title.len() as f32 * kit.line(TypeRole::Title);
    let head_h = ICON.max(title_h);
    let body = wrap::lines(kit, body_style, &spec.body, inner, 12);
    let body_h = body.len() as f32 * kit.line(TypeRole::Body);
    // the buttons go along a line and onto the next one when they do not fit
    let (bh, gap) = (Button::height(kit), s4);
    let mut at = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for a in &spec.answers {
        let w = Button::width_for(kit, &a.label, false);
        if x > 0.0 && x + w > inner {
            x = 0.0;
            y += bh + gap;
        }
        at.push((x, y, w));
        x += w + gap;
    }
    let buttons_h = at.last().map_or(0.0, |l| l.1 + bh);
    let body_top = head_h + if body.is_empty() { 0.0 } else { s4 };
    let buttons_top = body_top + body_h + if at.is_empty() { 0.0 } else { s4 };
    let h = (s6 + buttons_top + buttons_h + s6).ceil();
    let card = Frame::new(((1920.0 - CARD_W) / 2.0).round(), ((1080.0 - h) / 2.0).round(), CARD_W, h);
    let (cx, cy) = (card.x + s6, card.y + s6);
    Layout {
        card,
        icon: Frame::new(cx, cy + ((head_h - ICON) / 2.0).round(), ICON, ICON),
        title_at: (cx + ICON + s4, cy + ((head_h - title_h) / 2.0).round(), title_w),
        title,
        body_at: (cx, cy + body_top, inner),
        body,
        buttons: at.into_iter().map(|(x, y, w)| Frame::new(cx + x, cy + buttons_top + y, w, bh)).collect(),
    }
}

struct Built {
    sig: String,
    _card: ShapeLayer,
    _icon: Option<ImageLayer>,
    title: Words,
    body: Words,
    buttons: Vec<Button>,
    lay: Layout,
    answers: Vec<(String, bool)>,
    layers: usize,
}

pub struct ConfirmSurface {
    stage: Stage,
    built: Option<Built>,
}

impl ConfirmSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).hiding(), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, spec: &Spec, lay: &Layout) -> String {
        format!("{}|{}|{:?}|{:?}", kit.generation(), kit.text_scale, spec, lay.card)
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, spec: &Spec, lay: Layout, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let (ink, ink2, sheet) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.colour(PaletteRole::Sheet));
        let shape = Shape { corner_radius: kit.tokens.radius(Radius::Sheet), fill: Fill::Solid(sheet), border_width: 1.0, border: edge(kit) };
        scene
            .adopting_view(root, || -> std::io::Result<Built> {
                let card = plate::shape(&kit.ui, lay.card, z + 1, shape);
                let icon = if spec.icon_url.is_empty() {
                    let name = if spec.icon.is_empty() { "circle-help" } else { spec.icon.as_str() };
                    Some(tile(kit, ICON as u32, name, true).layer(kit, z + 2, Props::at(lay.icon.x, lay.icon.y))?)
                } else {
                    picture_layer(kit, &spec.icon_url, ICON as u32, ICON as u32, z + 2, Props::at(lay.icon.x, lay.icon.y))?
                };
                let title_lines: Vec<WordLine> = lay.title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
                let title = Words::new(kit, lay.title_at.0, lay.title_at.1, lay.title_at.2, z + 2, &title_lines)?;
                let body_lines: Vec<WordLine> = lay.body.iter().map(|t| WordLine::new(t, TypeRole::Body, ink2)).collect();
                let body = Words::new(kit, lay.body_at.0, lay.body_at.1, lay.body_at.2, z + 2, &body_lines)?;
                let mut buttons = Vec::new();
                for (i, (a, f)) in spec.answers.iter().zip(&lay.buttons).enumerate() {
                    // the safe answer (the one that is focused first) is the task's one primary button
                    let tone = if a.danger {
                        Tone::Danger
                    } else if i == spec.focus.min(spec.answers.len().saturating_sub(1)) {
                        Tone::Primary
                    } else {
                        Tone::Plain
                    };
                    let mut b = Button::new(
                        kit,
                        engine,
                        Place { id: Target::Answer(i).id(), frame: *f, region: REGION, scope: SCOPE, z: z + 3 },
                        ButtonSpec::new(&a.label, tone),
                    )?;
                    b.show(kit, true);
                    buttons.push(b);
                }
                let layers = before.saturating_sub(kit.room().layers);
                Ok(Built {
                    sig,
                    _card: card,
                    _icon: icon,
                    title,
                    body,
                    buttons,
                    lay: lay.clone(),
                    answers: spec.answers.iter().map(|a| (a.label.clone(), a.danger)).collect(),
                    layers,
                })
            })
            .map_err(std::io::Error::other)
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for btn in b.buttons {
                btn.destroy(engine);
            }
        }
    }

    /// Fits the dialog to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let lay = layout(kit, &v.spec);
        if self.stage.needs_root(Arrive::Fade) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, Arrive::Fade, true)?;
        let sig = Self::signature(kit, &v.spec, &lay);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), &v.spec, lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the confirm dialog could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, phase) {
            self.stage.focus(engine, (!v.spec.answers.is_empty()).then(|| Target::Answer(v.index).id()), now_ns);
        }
        Ok(true)
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue and the press follow it on the buttons, and a click is the answer it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let built = &mut self.built;
        let current = engine.focus.current();
        let mut set = |id: Id, f: &dyn Fn(&mut Button)| {
            if let (Some(b), Some(Target::Answer(i))) = (built.as_mut(), Target::from_id(id))
                && let Some(btn) = b.buttons.get_mut(i)
            {
                f(btn);
            }
        };
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| {
            match touch {
                Touch::Hover { from, to } => {
                    if let Some(f) = from {
                        set(f, &|b| b.set_hover(kit, false));
                    }
                    if let Some(t) = to.filter(|t| current != Some(*t)) {
                        set(t, &|b| b.set_hover(kit, true));
                    }
                }
                Touch::Press { id, down } => set(id, &|b| b.set_pressed(kit, down)),
            }
            None
        })?;
        Target::from_id(click)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    /// How many layers the dialog holds.
    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers + 1)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let answers: Vec<Value> = b
            .buttons
            .iter()
            .zip(&b.answers)
            .enumerate()
            .map(|(i, (btn, (label, danger)))| json!({"frame": f(engine.focus.frame(btn.id).unwrap_or(btn.frame())), "label": label, "danger": danger, "i": i}))
            .collect();
        json!({"card": f(b.lay.card), "answers": answers, "focus": engine.focus.current().and_then(Target::from_id).map(|Target::Answer(i)| i), "layers": self.layers(), "scope": SCOPE.0})
    }

    /// Describes what is on the screen to a lint: the card, the words on it and each answer's button.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let card = lint.panel("confirm card", b.lay.card, kit.ui.look.colour(PaletteRole::Sheet), false);
        b.title.feed_lint(kit, lint, "confirm title", Some(card), (0.0, 0.0));
        b.body.feed_lint(kit, lint, "confirm body", Some(card), (0.0, 0.0));
        for btn in &b.buttons {
            btn.feed_lint(kit, lint, card);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::confirm::Confirm;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: ConfirmSurface,
        c: Confirm,
        t: u64,
    }

    fn uninstall() -> Value {
        json!({
            "icon": "trash-2", "title": "Uninstall Crunchyroll?", "body": "The app and its data are removed from this TV. Your account stays where it is and the app can be installed again.",
            "answers": [{"id": "keep", "label": "Keep it"}, {"id": "uninstall", "label": "Uninstall", "danger": true}], "focus": 0
        })
    }

    fn many() -> Value {
        json!({
            "title": "How should the TV start next time, and do you want to keep what is on the screen now?", "body": "",
            "answers": [{"id": "a", "label": "Start on Home"}, {"id": "b", "label": "Start on the last input"}, {"id": "c", "label": "Start on Live TV with the guide"}, {"id": "d", "label": "Ask me"}], "focus": 1
        })
    }

    impl Up {
        fn new(mode: Mode, scale: f32, spec: Value) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut c = Confirm::new();
            c.life.show(0);
            c.ask(Spec::from_json(&spec));
            let mut me = Self { _h: h, kit, engine, s: ConfirmSurface::new(crate::kit::z::PANELS), c, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.c.view(false).expect("a view");
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.c.life.phase(), self.t * 1_000_000).expect("the dialog is drawn");
        }

        fn key(&mut self, k: K) {
            self.c.key(k);
            self.t += 40;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }
    }

    fn at(x: f32, y: f32, b: u32) -> Sample {
        Sample { x, y, buttons: b, visible: true }
    }

    #[test]
    fn the_card_is_760_wide_in_the_middle_with_a_button_per_answer_and_the_safe_one_focused_first() {
        let u = Up::new(Mode::Dark, 1.0, uninstall());
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["card"]["w"], 760.0);
        assert_eq!(g["card"]["x"], 580.0);
        let (y, h) = (g["card"]["y"].as_f64().unwrap(), g["card"]["h"].as_f64().unwrap());
        assert!((y + h / 2.0 - 540.0).abs() <= 1.0, "centred: {y} {h}");
        let a = g["answers"].as_array().unwrap();
        assert_eq!(a.len(), 2);
        assert!(a[0]["frame"]["x"].as_f64().unwrap() < a[1]["frame"]["x"].as_f64().unwrap() && a[1]["danger"] == true);
        assert_eq!(g["focus"], 0, "Keep it");
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
    }

    #[test]
    fn the_focus_flies_between_the_answers_with_no_layer_made() {
        let mut u = Up::new(Mode::Dark, 1.0, uninstall());
        let layers = u.kit.room().layers;
        u.key(K::Right);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 1);
        assert!(u.engine.flow.settling_at(u.t * 1_000_000));
        u.key(K::Left);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 0);
        assert_eq!(u.kit.room().layers, layers);
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_with_few_answers_and_many() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for (name, spec) in [("uninstall", uninstall()), ("many", many())] {
                    let mut u = Up::new(mode, scale, spec);
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} {mode:?} {scale}: {f:#?}");
                    let c = &u.s.geometry(&u.engine)["card"];
                    assert!(
                        c["y"].as_f64().unwrap() >= 0.0 && c["y"].as_f64().unwrap() + c["h"].as_f64().unwrap() <= 1080.0,
                        "{name} {scale}: inside the screen: {c}"
                    );
                }
            }
        }
    }

    #[test]
    fn answers_that_do_not_fit_on_a_line_go_on_to_the_next() {
        let u = Up::new(Mode::Dark, 1.0, many());
        let a = u.s.geometry(&u.engine)["answers"].clone();
        let ys: Vec<f64> = a.as_array().unwrap().iter().map(|x| x["frame"]["y"].as_f64().unwrap()).collect();
        assert!(ys.last() > ys.first(), "{ys:?}");
        assert_eq!(u.s.geometry(&u.engine)["focus"], 1);
    }

    #[test]
    fn a_click_on_an_answer_names_it_and_the_dim_is_no_target() {
        let mut u = Up::new(Mode::Dark, 1.0, uninstall());
        let f = u.engine.focus.frame(Target::Answer(1).id()).expect("a button");
        let t = |n: u64| (3000 + n) * 1_000_000;
        u.s.pointer(&u.kit, &mut u.engine, t(0), at(f.x + 10.0, f.y + 10.0, 0));
        assert!(!u.s.stage.hover.is_shown(), "a button takes its cue itself: the plate cue is not used");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(f.x + 10.0, f.y + 10.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(f.x + 10.0, f.y + 10.0, 0)), Some(Target::Answer(1)));
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(60.0, 60.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(60.0, 60.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(50), at(60.0, 60.0, 0)), None, "a click on the dim does nothing");
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Answer(0), Target::Answer(3)] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
        assert_eq!(Target::from_id(Id(1)), None);
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, uninstall());
        let with = u.kit.room().layers;
        u.c.close();
        u.c.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.c.life.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.c.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 10 && !u.s.is_up());
    }
}
