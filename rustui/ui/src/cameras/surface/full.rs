//! The full view drawn (`docs/mocks/cameras-scrypted/full*.jpg`, spec "The full view"): the camera on the whole screen, fitted, with black beside
//! it; where the stream plays on the video plane the layer is a **hole** (nothing is drawn where the picture is, so the video shows through)
//! and the chrome stays above. The chrome is one group on a reading surface: the name pill (`video`, the name, the dot and the state's word,
//! "2 of 8") at the top left, the info pill (`3840x2160 . H.265 . 15 fps`) at the top right, "Opens in a few seconds" while a stream that is
//! not prebuffered starts, the hints (glyphs) at the bottom right. It fades out 2.5 s after the last key (180 ms) and in over 120 ms. A camera
//! that cannot play shows a note over a dimmed picture: the title, the reason and a `Try again` button.

use super::common::{Ink, Parts, scale};
use super::grid::WordRec;
use super::pic::{Fill, PicLayer};
use super::pill::{self, Anchor, Piece};
use super::{Cx, REGION_FULL, SCOPE_BASE, depth, ids};
use crate::cameras::app::Target;
use crate::cameras::view::{Backdrop, FullView, HintView, Note};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::focus::Id;
use crate::kit::hint::Key;
use crate::kit::icon;
use crate::kit::pane::{Pane, PaneKind, glass};
use crate::kit::text::{Text, role_style};
use crate::kit::wrap;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::scene::ShapeLayer;
use ui::text::Weight;

/// The chrome fades out over this, and in over this (ms).
pub const FADE_OUT_MS: u32 = 180;
pub const FADE_IN_MS: u32 = 120;

/// The veil colour over a still: dark ink over the picture so the chrome reads against it.
fn veil(kit: &crate::kit::Kit, alpha: f32) -> Rgba {
    let (r, g, b) = match kit.mode {
        ui::lucent::palette::Mode::Dark => (8.0, 6.0, 16.0),
        ui::lucent::palette::Mode::Light => (8.0, 6.0, 16.0),
    };
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round();
    Rgba((r * alpha) as u8, (g * alpha) as u8, (b * alpha) as u8, a as u8)
}

struct Piece2 {
    sig: String,
    parts: Parts,
    drawn: pill::Drawn,
}

struct NoteUi {
    sig: String,
    parts: Parts,
    frame: Frame,
    paint: Rgba,
    words: Vec<WordRec>,
    button: Button,
}

pub struct FullUi {
    ink: Ink,
    sig: String,
    black: ShapeLayer,
    veil_layer: ShapeLayer,
    veil_alpha: f32,
    still: Option<PicLayer>,
    still_key: Option<String>,
    name: Option<Piece2>,
    info: Option<Piece2>,
    line: Option<Piece2>,
    hints: Option<Piece2>,
    note: Option<NoteUi>,
    chrome_on: bool,
    hole: bool,
}

fn signature(cx: &Cx, view: &FullView) -> String {
    format!("{}|{}|{}", cx.kit.generation(), cx.kit.text_scale, view.name.len() + view.position.as_ref().map_or(0, String::len))
}

impl FullUi {
    pub fn build(cx: &mut Cx, view: &FullView) -> std::io::Result<Self> {
        let ink = Ink::of(cx.kit);
        let black = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), cx.at(depth::BLACK), plate::solid(Rgba(0, 0, 0, 255), 0.0));
        let veil_layer = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), cx.at(depth::VEIL), plate::solid(veil(cx.kit, 0.5), 0.0));
        veil_layer.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
        // a click on the picture brings the chrome back (the lowest focusable: later ones are over it)
        cx.engine.focus.register(ids::BACKDROP, Frame::new(0.0, 0.0, 1920.0, 1080.0), REGION_FULL, SCOPE_BASE);
        let mut me = Self {
            ink,
            sig: signature(cx, view),
            black,
            veil_layer,
            veil_alpha: -1.0,
            still: None,
            still_key: None,
            name: None,
            info: None,
            line: None,
            hints: None,
            note: None,
            chrome_on: view.chrome,
            hole: false,
        };
        me.sync(cx, view)?;
        if cx.kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the full view could not be drawn whole"));
        }
        Ok(me)
    }

    pub fn fits(&self, cx: &Cx, view: &FullView) -> bool {
        let _ = view;
        self.sig.starts_with(&format!("{}|{}|", cx.kit.generation(), cx.kit.text_scale))
    }

    pub fn retire(self, engine: &mut crate::kit::Engine) {
        engine.focus.unregister(ids::BACKDROP);
        if let Some(n) = &self.note {
            n.button.detach(engine);
        }
    }

    /// Follows the view: the picture and its veil, the hole, the pieces of the chrome and their fade, the note.
    pub fn sync(&mut self, cx: &mut Cx, view: &FullView) -> std::io::Result<()> {
        // the hole: nothing is drawn where the picture is while the stream plays on the video plane
        if view.hole != self.hole || self.veil_alpha < 0.0 {
            self.hole = view.hole;
            self.black.set(if view.hole { Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) } } else { Props::at(0.0, 0.0) });
        }
        // the picture
        let want_key = match &view.backdrop {
            Backdrop::Still(k) | Backdrop::Thumb(k) => Some(k.clone()),
            Backdrop::None => None,
        };
        if want_key != self.still_key {
            self.still = want_key.as_ref().map(|k| PicLayer::new(k, Frame::new(0.0, 0.0, 1920.0, 1080.0), 0.0, Fill::Contain, cx.at(depth::PICTURE)));
            self.still_key = want_key;
        }
        if let Some(p) = &mut self.still {
            p.sync(cx.kit, None, cx.store)?;
        }
        if (view.veil - self.veil_alpha).abs() > 0.001 {
            self.veil_alpha = view.veil;
            if view.veil > 0.0 {
                self.veil_layer.reshape(plate::solid(veil(cx.kit, view.veil), 0.0));
                self.veil_layer.set(Props::at(0.0, 0.0));
            } else {
                self.veil_layer.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
            }
        }
        // the chrome's pieces, each made again only when it says something else
        self.sync_name(cx, view)?;
        self.sync_info(cx, view)?;
        self.sync_line(cx, view)?;
        self.sync_hints(cx, view)?;
        self.sync_note(cx, view)?;
        // the fade
        if view.chrome != self.chrome_on {
            self.chrome_on = view.chrome;
            let ms = if view.chrome { FADE_IN_MS } else { FADE_OUT_MS };
            for p in [&mut self.name, &mut self.info, &mut self.line, &mut self.hints].into_iter().flatten() {
                p.parts.fade(cx.kit, view.chrome, ms);
            }
        }
        match (&self.note, view.button_focused) {
            (Some(n), true) => cx.follow(n.button.id),
            _ => {
                if cx.engine.focus.current().is_some() && !view.button_focused {
                    cx.engine.clear(cx.now_ns);
                }
            }
        }
        Ok(())
    }

    fn piece(cx: &mut Cx, slot: &mut Option<Piece2>, sig: Option<String>, on: bool, build: impl FnOnce(&mut Cx) -> std::io::Result<(Parts, pill::Drawn)>) -> std::io::Result<()> {
        if slot.as_ref().map(|p| p.sig.clone()) == sig {
            return Ok(());
        }
        *slot = None;
        if let Some(sig) = sig {
            let (mut parts, drawn) = build(cx)?;
            parts.fade(cx.kit, on, 0);
            *slot = Some(Piece2 { sig, parts, drawn });
        }
        Ok(())
    }

    fn sync_name(&mut self, cx: &mut Cx, view: &FullView) -> std::io::Result<()> {
        let sig = format!("{}|{}|{}|{:?}", view.name, view.position.clone().unwrap_or_default(), view.state.word, view.state.dot);
        let ink = self.ink;
        let (v, on) = (view.clone(), view.chrome);
        Self::piece(cx, &mut self.name, Some(sig), on, move |cx| {
            let mut pieces = vec![
                Piece::Icon { name: "video", base: 32.0, ink: ink.ink },
                Piece::Text { text: &v.name, role: TypeRole::Body, weight: Some(Weight::SemiBold), ink: ink.ink },
            ];
            if v.state.dot {
                pieces.push(Piece::Dot(ink.live));
            }
            pieces.push(Piece::Text { text: v.state.word, role: TypeRole::Body, weight: Some(Weight::SemiBold), ink: ink.ink });
            if let Some(pos) = &v.position {
                pieces.push(Piece::Text { text: pos, role: TypeRole::Label, weight: None, ink: ink.ink2 });
            }
            let z = cx.at(depth::CHROME);
            pill::build(cx.kit, "name pill", &pieces, Anchor::TopLeft { x: 96.0, y: 54.0 }, z, &ink)
        })
    }

    fn sync_info(&mut self, cx: &mut Cx, view: &FullView) -> std::io::Result<()> {
        let sig = view.info.clone();
        let ink = self.ink;
        let on = view.chrome;
        Self::piece(cx, &mut self.info, sig.clone(), on, move |cx| {
            let text = sig.unwrap_or_default();
            let pieces = [Piece::Text { text: &text, role: TypeRole::Caption, weight: None, ink: ink.ink2 }];
            let z = cx.at(depth::CHROME);
            pill::build(cx.kit, "info pill", &pieces, Anchor::TopRight { right: 1824.0, y: 54.0 }, z, &ink)
        })
    }

    fn sync_line(&mut self, cx: &mut Cx, view: &FullView) -> std::io::Result<()> {
        let sig = view.line.clone();
        let ink = self.ink;
        let on = view.chrome;
        Self::piece(cx, &mut self.line, sig.clone(), on, move |cx| {
            let text = sig.unwrap_or_default();
            let pieces = [Piece::Text { text: &text, role: TypeRole::Label, weight: None, ink: ink.ink2 }];
            let z = cx.at(depth::CHROME);
            pill::build(cx.kit, "line pill", &pieces, Anchor::BottomLeft { x: 96.0, bottom: 1026.0 }, z, &ink)
        })
    }

    fn sync_hints(&mut self, cx: &mut Cx, view: &FullView) -> std::io::Result<()> {
        let sig = (!view.hints.is_empty()).then(|| view.hints.iter().map(|h| h.name.as_str()).collect::<Vec<_>>().join("|"));
        let ink = self.ink;
        let hints: Vec<HintView> = view.hints.clone();
        let on = view.chrome;
        Self::piece(cx, &mut self.hints, sig, on, move |cx| {
            let z = cx.at(depth::CHROME);
            pill::build(cx.kit, "hints pill", &[Piece::Hints(&hints)], Anchor::BottomRight { right: 1824.0, bottom: 1026.0 }, z, &ink)
        })
    }

    fn sync_note(&mut self, cx: &mut Cx, view: &FullView) -> std::io::Result<()> {
        let sig = view.note.as_ref().map(|n| format!("{}|{}|{}", n.title, n.detail, cx.kit.text_scale));
        if self.note.as_ref().map(|n| n.sig.clone()) == sig {
            return Ok(());
        }
        if let Some(old) = self.note.take() {
            old.button.detach(cx.engine);
        }
        let Some(note) = &view.note else { return Ok(()) };
        self.note = Some(self.build_note(cx, note, sig.unwrap_or_default())?);
        Ok(())
    }

    fn build_note(&mut self, cx: &mut Cx, note: &Note, sig: String) -> std::io::Result<NoteUi> {
        let ts = scale(cx.kit);
        let (pad_x, pad_y) = (44.0, 36.0);
        let width = 960.0;
        let inner = width - 2.0 * pad_x;
        let z = cx.at(depth::NOTE);
        let title_style = role_style(cx.kit, TypeRole::Title, Some(Weight::SemiBold), self.ink.ink, Align::Centre);
        let detail_style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink2, Align::Centre);
        let title_lines = wrap::lines(cx.kit, title_style, note.title, inner, 2);
        let detail_lines = wrap::lines(cx.kit, detail_style, &note.detail, inner, 3);
        let (tl, dl) = (cx.kit.line(TypeRole::Title), cx.kit.line(TypeRole::Label));
        let side = (56.0 * (1.0 + (ts - 1.0) * 0.5)).round() as u32;
        let btn_h = Button::height(cx.kit);
        let btn_w = Button::width_for(cx.kit, note.button, false);
        let hint = HintView::new(&[Key::Back], crate::cameras::view::Do::Kit(crate::kit::hint::Action::Close), "Back, close");
        let hint_w = pill::hints_width(cx.kit, std::slice::from_ref(&hint));
        let hint_side = pill::hint_side(cx.kit) as f32;
        let height = pad_y + side as f32 + 12.0 + title_lines.len() as f32 * tl + 10.0 + detail_lines.len() as f32 * dl + 22.0 + btn_h + pad_y;
        let frame = Frame::new(((1920.0 - width) / 2.0).round(), ((1080.0 - height) / 2.0).round(), width, height);
        let mut parts = Parts::new();
        let pane = Pane::new(cx.kit, frame, z, PaneKind::Card, true);
        let paint = glass(cx.kit, PaneKind::Card, height, true).tint;
        parts.pane(pane);
        let mut y = frame.y + pad_y;
        if let Some(layer) = icon::layer(cx.kit, "triangle-alert", side, self.ink.ink, z + 1, crate::system::parts::HIDDEN)? {
            parts.image(layer, (frame.x + (width - side as f32) / 2.0, y));
        }
        y += side as f32 + 12.0;
        let mut words = Vec::new();
        for l in &title_lines {
            let f = Frame::new(frame.x + pad_x, y, inner, tl);
            parts.text(Text::new(cx.kit, f, z + 1, title_style, l)?);
            words.push(WordRec { name: "note title".into(), text: l.clone(), frame: f, style: title_style });
            y += tl;
        }
        y += 10.0;
        for l in &detail_lines {
            let f = Frame::new(frame.x + pad_x, y, inner, dl);
            parts.text(Text::new(cx.kit, f, z + 1, detail_style, l)?);
            words.push(WordRec { name: "note detail".into(), text: l.clone(), frame: f, style: detail_style });
            y += dl;
        }
        y += 22.0;
        let row_w = btn_w + 28.0 + hint_w;
        let x0 = frame.x + (width - row_w) / 2.0;
        let place = Place { id: ids::RETRY, frame: Frame::new(x0.round(), y, btn_w, btn_h), region: REGION_FULL, scope: SCOPE_BASE, z: z + 2 };
        let radius = cx.kit.tokens.radius(Radius::Control) + 2.0;
        let mut button = Button::new(cx.kit, cx.engine, place, Spec::new(note.button, Tone::Primary).with_radius(radius))?;
        button.show(cx.kit, true);
        pill::draw_hints(cx.kit, std::slice::from_ref(&hint), x0 + btn_w + 28.0, y + btn_h / 2.0, z + 2, &self.ink, &mut parts)?;
        let _ = hint_side;
        parts.fade(cx.kit, true, 0);
        Ok(NoteUi { sig, parts, frame, paint, words, button })
    }

    /// The pointer's cue on `id`.
    pub fn hover(&mut self, cx: &mut Cx, id: Id, on: bool) {
        if let Some(n) = &mut self.note {
            if n.button.id == id {
                n.button.set_hover(cx.kit, on && cx.engine.focus.current() != Some(id));
            }
        }
    }

    pub fn press(&mut self, cx: &mut Cx, id: Id, down: bool) {
        if let Some(n) = &mut self.note {
            if n.button.id == id {
                n.button.set_pressed(cx.kit, down);
            }
        }
    }

    pub fn target(&self, id: Id) -> Option<Target> {
        if id == ids::RETRY && self.note.is_some() {
            return Some(Target::Retry);
        }
        (id == ids::BACKDROP).then_some(Target::Backdrop)
    }

    /// The layers the full view holds, for the budget.
    pub fn layers(&self) -> usize {
        let pieces: usize = [&self.name, &self.info, &self.line, &self.hints].into_iter().flatten().map(|p| p.parts.layers()).sum();
        2 + usize::from(self.still.as_ref().is_some_and(|p| p.layer().is_some())) + pieces + self.note.as_ref().map_or(0, |n| n.parts.layers() + n.button.layers())
    }

    /// Whether the chrome is on (faded in).
    pub fn chrome_on(&self) -> bool {
        self.chrome_on
    }

    /// The pills and the note as drawn, for the harness and the lint.
    pub fn rects(&self) -> Vec<(&'static str, Frame)> {
        let mut v = Vec::new();
        for (name, p) in [("name", &self.name), ("info", &self.info), ("line", &self.line), ("hints", &self.hints)] {
            if let Some(p) = p {
                v.push((name, p.drawn.frame));
            }
        }
        if let Some(n) = &self.note {
            v.push(("note", n.frame));
        }
        v
    }

    pub fn feed_lint(&self, kit: &mut crate::kit::Kit, lint: &mut crate::kit::lint::Lint) {
        for p in [&self.name, &self.info, &self.line, &self.hints].into_iter().flatten() {
            let panel = lint.panel(p.drawn.words.first().map_or("pill", |w| w.name.split(' ').next().unwrap_or("pill")), p.drawn.frame, p.drawn.paint, true);
            for w in &p.drawn.words {
                lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
            }
        }
        if let Some(n) = &self.note {
            let panel = lint.panel("note", n.frame, n.paint, false);
            for w in &n.words {
                lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
            }
            n.button.feed_lint(kit, lint, panel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::app::{App, Config};
    use crate::cameras::surface::pic::PictureStore;
    use crate::cameras::view::Base;
    use crate::kit::lint::Lint;
    use crate::kit::screen::{Input, Model};
    use crate::kit::testkit;
    use serde_json::{Value, json};

    fn camera(id: &str, name: &str) -> Value {
        json!({"id": id, "name": name, "online": true, "live": true, "recordings": true, "events": true, "clips": false, "prebuffered": true,
               "native": {"codec": "h265", "width": 3840, "height": 2160, "fps": 15}})
    }

    fn app(text_scale: f32) -> App {
        let mut a = App::new(Config { text_scale, ..Config::default() });
        a.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "ready", "cameras": [camera("273", "Front Door"), camera("274", "Back Door")]}) }, 1000);
        a
    }

    fn full(a: &App) -> FullView {
        match a.view().base {
            Base::Full(f) => f,
            other => panic!("not the full view: {other:?}"),
        }
    }

    fn cx<'a>(kit: &'a mut crate::kit::Kit, engine: &'a mut crate::kit::Engine, store: &'a PictureStore) -> Cx<'a> {
        Cx { kit, engine, store, z: 2000, now_ms: 1000, now_ns: 1_000_000_000 }
    }

    fn live(state: &str) -> Input {
        Input::Helper {
            topic: "cameras.live".into(),
            data: json!({"camera": "273", "mode": "live", "state": state, "reason": "The camera is offline.", "codec": "h265", "width": 3840, "height": 2160, "fps": 15,
                         "startedAt": null, "positionMs": null, "rate": 1, "paused": false}),
        }
    }

    #[test]
    fn a_stream_that_plays_is_a_hole_with_the_chrome_above_and_black_when_it_does_not() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut a = app(1.0);
        a.open_full("273", 1000);
        let mut f = FullUi::build(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        assert!(!f.hole, "asked for, not playing: black");
        assert!(f.name.is_some() && f.hints.is_some() && f.info.is_some());
        a.update(&live("playing"), 1100);
        f.sync(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        assert!(f.hole, "the layer is transparent over the video plane");
        assert!(f.name.as_ref().unwrap().sig.contains("Live"));
        assert!(f.info.as_ref().unwrap().sig.contains("3840"));
        assert!(f.still.is_none(), "no picture is drawn where the stream plays");
    }

    #[test]
    fn the_pills_sit_where_the_mock_puts_them_and_the_chrome_fades_with_the_view() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut a = app(1.0);
        a.open_full("273", 1000);
        a.update(&live("playing"), 1050);
        let mut f = FullUi::build(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        let rects: std::collections::HashMap<_, _> = f.rects().into_iter().collect();
        assert_eq!((rects["name"].x, rects["name"].y), (96.0, 54.0));
        assert!((rects["info"].right() - 1824.0).abs() < 0.01 && rects["info"].y == 54.0);
        assert!((rects["hints"].right() - 1824.0).abs() < 0.01 && (rects["hints"].bottom() - 1026.0).abs() < 0.01);
        assert!(f.chrome_on());
        a.update(&Input::Tick, 1000 + 2600);
        f.sync(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        assert!(!f.chrome_on(), "2.5 s without a key");
    }

    #[test]
    fn a_failure_shows_the_note_over_a_veil_with_a_button_that_is_the_focus() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut a = app(1.0);
        a.open_full("273", 1000);
        a.update(&live("failed"), 1100);
        let mut f = FullUi::build(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        f.sync(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        assert!(f.note.is_some());
        assert_eq!(engine.focus.current(), Some(ids::RETRY));
        assert_eq!(f.target(ids::RETRY), Some(Target::Retry));
        assert_eq!(f.target(ids::BACKDROP), Some(Target::Backdrop));
        assert!(f.veil_alpha > 0.5);
        // the camera plays again: the note goes, and so does its button
        a.update(&live("playing"), 1200);
        f.sync(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
        assert!(f.note.is_none());
        assert!(!engine.focus.is_live(ids::RETRY));
    }

    #[test]
    fn the_full_view_reads_clean_to_the_lint_at_both_text_sizes_and_in_both_looks() {
        for scale in [1.0f32, 2.0] {
            for mode in [ui::lucent::palette::Mode::Dark, ui::lucent::palette::Mode::Light] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = crate::kit::Engine::new(&kit).unwrap();
                let store = PictureStore::new();
                for state in ["playing", "failed", "starting"] {
                    let mut a = app(scale);
                    a.open_full("273", 1000);
                    a.update(&live(state), 1050);
                    let mut f = FullUi::build(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
                    f.sync(&mut cx(&mut kit, &mut engine, &store), &full(&a)).unwrap();
                    let mut lint = Lint::new().with_safe(Frame::new(96.0, 54.0, 1728.0, 972.0));
                    f.feed_lint(&mut kit, &mut lint);
                    let findings: Vec<String> = lint.findings().iter().map(|x| x.to_string()).collect();
                    assert!(findings.is_empty(), "{state} at {scale} in {mode:?}: {findings:?}");
                    f.retire(&mut engine);
                }
            }
        }
    }
}
