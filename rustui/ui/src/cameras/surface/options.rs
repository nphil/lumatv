//! The per-camera pop-up options page drawn (`docs/mocks/cameras-scrypted/settings-notify-cam*.jpg`): a sheet over the grid with a way back
//! (`Cameras`), the camera's name, the group label "Pop up when it sees" and a row for each kind, each a quiet tile with the kind's glyph,
//! its name and a switch. It is the page of the helper's schema `scrypted-cameras` (`notify.<id>`) in the form of the Cameras app; Settings
//! draws the same page from the schema with its own page renderer, and a switch in either is a write of the field's list.
//!
//! A row is a focusable of the engine (it lifts with the flowing focus, 1.02, and wears the raised wash); "on" is never colour alone: the
//! switch's knob sits at the right and carries a tick.

use super::common::{Ink, Parts, icon_side, scale};
use super::grid::{WordRec, other};
use super::{Cx, REGION_OPTIONS, SCOPE_OPTIONS, depth, ids};
use crate::cameras::app::Target;
use crate::cameras::view::{OptionRow, OptionsView};
use crate::kit::button::{Button, Place, Spec, Tone, faded};
use crate::kit::flow::Lifted;
use crate::kit::focus::Id;
use crate::kit::icon;
use crate::kit::pane::{Pane, PaneKind, glass};
use crate::kit::text::{Text, role_style, width_of};
use crate::system::parts::{HIDDEN, icon_tile, tile_glyph_at};
use compositor_protocol::{GroupPose, Props, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const WIDTH: f32 = 960.0;
const TRACK_W: f32 = 76.0;
const TRACK_H: f32 = 44.0;
const KNOB: f32 = 36.0;

struct RowUi {
    id: Id,
    kind: &'static str,
    frame: Frame,
    body: Group,
    track: ShapeLayer,
    knob: ShapeLayer,
    tick: Option<ImageLayer>,
    on: bool,
    track_at: (f32, f32),
    _keep: Vec<Box<dyn std::any::Any>>,
}

pub struct OptionsUi {
    ink: Ink,
    sig: String,
    frame: Frame,
    paint: Rgba,
    parts: Parts,
    words: Vec<WordRec>,
    back: Button,
    rows: Vec<RowUi>,
    modal: bool,
}

fn signature(cx: &Cx, view: &OptionsView) -> String {
    format!("{}|{}|{}", cx.kit.generation(), cx.kit.text_scale, view.title)
}

impl OptionsUi {
    pub fn build(cx: &mut Cx, view: &OptionsView) -> std::io::Result<Self> {
        let ink = Ink::of(cx.kit);
        let ts = scale(cx.kit);
        let z = cx.at(depth::OPTIONS);
        let (pad_x, pad_y) = (40.0, 32.0);
        let title_style = role_style(cx.kit, TypeRole::Title, Some(Weight::SemiBold), ink.ink, Align::Left);
        let group_style = role_style(cx.kit, TypeRole::Label, None, ink.ink2, Align::Left);
        let (tl, gl) = (cx.kit.line(TypeRole::Title), cx.kit.line(TypeRole::Label));
        let row_h = 80.0f32.max(cx.kit.line(TypeRole::Body) + 2.0 * cx.kit.space(2)).ceil();
        let head_h = Button::height(cx.kit).max(tl);
        let height = pad_y + head_h + 16.0 + gl + 8.0 + view.rows.len() as f32 * row_h + pad_y;
        let frame = Frame::new(((1920.0 - WIDTH) / 2.0).round(), ((1080.0 - height) / 2.0).round(), WIDTH, height);
        let paint = glass(cx.kit, PaneKind::Sheet, height, false).tint;
        let mut parts = Parts::new();
        parts.pane(Pane::new(cx.kit, frame, z, PaneKind::Sheet, false));
        cx.engine.focus.open_modal(SCOPE_OPTIONS);
        let mut words = Vec::new();
        // the way back, then the camera's name
        let back_label = view.back;
        let bw = Button::width_for(cx.kit, back_label, true);
        let bh = Button::height(cx.kit);
        let cy = frame.y + pad_y + head_h / 2.0;
        let place = Place { id: ids::OPTIONS_BACK, frame: Frame::new(frame.x + pad_x, (cy - bh / 2.0).round(), bw, bh), region: REGION_OPTIONS, scope: SCOPE_OPTIONS, z: z + 3 };
        let mut back = Button::new(cx.kit, cx.engine, place, Spec::new(back_label, Tone::Plain).with_icon("chevron-left"))?;
        back.show(cx.kit, true);
        let title_w = width_of(cx.kit, title_style, &view.title).min(frame.w - 2.0 * pad_x - bw - 20.0);
        let title_f = Frame::new(frame.x + pad_x + bw + 20.0, (cy - tl / 2.0).round(), title_w, tl);
        parts.text(Text::new(cx.kit, title_f, z + 1, title_style, &view.title)?);
        words.push(WordRec { name: "options title".into(), text: view.title.clone(), frame: title_f, style: title_style });
        let group_f = Frame::new(frame.x + pad_x, frame.y + pad_y + head_h + 16.0, frame.w - 2.0 * pad_x, gl);
        parts.text(Text::new(cx.kit, group_f, z + 1, group_style, view.group)?);
        words.push(WordRec { name: "group".into(), text: view.group.to_string(), frame: group_f, style: group_style });
        let mut me = Self { ink, sig: signature(cx, view), frame, paint, parts, words, back, rows: Vec::new(), modal: true };
        let rows_top = group_f.bottom() + 8.0;
        for (i, row) in view.rows.iter().enumerate() {
            let r = me.build_row(cx, i, row, rows_top + i as f32 * row_h, row_h)?;
            me.rows.push(r);
        }
        let _ = ts;
        me.sync(cx, view)?;
        if cx.kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the options page could not be drawn whole"));
        }
        me.parts.fade(cx.kit, true, super::full::FADE_IN_MS);
        Ok(me)
    }

    pub fn fits(&self, cx: &Cx, view: &OptionsView) -> bool {
        self.sig == signature(cx, view)
    }

    fn build_row(&mut self, cx: &mut Cx, i: usize, row: &OptionRow, y: f32, h: f32) -> std::io::Result<RowUi> {
        let z = cx.at(depth::OPTIONS) + 10;
        let w = self.frame.w - 2.0 * 24.0;
        let x = self.frame.x + 24.0;
        let radius = cx.kit.tokens.radius(Radius::Row);
        let scene = cx.kit.ui.scene.clone();
        let rest = GroupPose { x, y, pivot_x: w / 2.0, pivot_y: h / 2.0, ..GroupPose::IDENTITY };
        let lift = scene.group(None, rest).map_err(other)?;
        let lit = scene.group(Some(lift.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let body = scene.group(Some(lift.id()), GroupPose::IDENTITY).map_err(other)?;
        let tile_s = 56.0;
        let style = role_style(cx.kit, TypeRole::Body, None, self.ink.ink, Align::Left);
        let line = cx.kit.line(TypeRole::Body);
        let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
        let on = row.on;
        let (track, knob, tick) = scene.adopting(&body, || -> std::io::Result<_> {
            let (plate_layer, glyph) = icon_tile(cx.kit, tile_s, z, row.icon, false)?;
            plate_layer.set(Props::at(16.0, ((h - tile_s) / 2.0).round()));
            keep.push(Box::new(plate_layer));
            if let Some(g) = glyph {
                let at = tile_glyph_at((16.0, ((h - tile_s) / 2.0).round()), tile_s);
                g.set(Props::at(at.0, at.1));
                keep.push(Box::new(g));
            }
            keep.push(Box::new(Text::new(cx.kit, Frame::new(16.0 + tile_s + 20.0, ((h - line) / 2.0).round(), w - 16.0 - tile_s - 20.0 - TRACK_W - 40.0, line), z + 1, style, row.name)?));
            let track_at = (w - 16.0 - TRACK_W, ((h - TRACK_H) / 2.0).round());
            let track = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, TRACK_W, TRACK_H), z + 1, plate::solid(Rgba(0, 0, 0, 0), TRACK_H / 2.0));
            track.set(Props::at(track_at.0, track_at.1));
            let knob = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, KNOB, KNOB), z + 2, plate::solid(Rgba(255, 255, 255, 255), KNOB / 2.0));
            let tick_side = icon_side(cx.kit, 20.0);
            let tick = icon::layer(cx.kit, "check", tick_side, self.ink.accent, z + 3, HIDDEN)?;
            Ok((track, knob, tick))
        })?;
        let lit_wash = scene.adopting(&lit, || -> Vec<ShapeLayer> {
            vec![
                plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, h), z - 1, plate::solid(faded(self.ink.ink, 0.07), radius)),
                plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, h), z, plate::ring(self.ink.edge_raised, radius, 1.5)),
            ]
        });
        for l in lit_wash {
            keep.push(Box::new(l));
        }
        let id = ids::option(i);
        let frame = Frame::new(x, y, w, h);
        cx.engine.focus.register(id, frame, REGION_OPTIONS, SCOPE_OPTIONS);
        cx.engine.flow.lift(id, Lifted { group: lift, rest, scale: cx.kit.tokens.focus.row_scale, rise: 0.0, lit: Some(lit) });
        let track_at = (w - 16.0 - TRACK_W, ((h - TRACK_H) / 2.0).round());
        let mut r = RowUi { id, kind: row.kind, frame, body, track, knob, tick, on: !on, track_at, _keep: keep };
        Self::paint_switch(&self.ink, cx, &mut r, on);
        Ok(r)
    }

    /// The switch as `on` says: the accent track with the knob at the right and a tick on it, or the quiet track with the knob at the left.
    fn paint_switch(ink: &Ink, cx: &Cx, r: &mut RowUi, on: bool) {
        if r.on == on {
            return;
        }
        r.on = on;
        let off = cx.kit.ui.look.colour(PaletteRole::TrackOff);
        let colour = if on { ink.accent } else { off };
        r.track.reshape(plate::solid(colour, TRACK_H / 2.0));
        let inset = (TRACK_H - KNOB) / 2.0;
        let kx = if on { r.track_at.0 + TRACK_W - KNOB - inset } else { r.track_at.0 + inset };
        r.knob.set(Props::at(kx, r.track_at.1 + inset));
        if let Some(t) = &r.tick {
            let s = t.width() as f32;
            let at = Props::at(kx + (KNOB - s) / 2.0, r.track_at.1 + inset + (KNOB - s) / 2.0);
            t.set(if on { at } else { Props { opacity: 0.0, visible: false, ..at } });
        }
    }

    pub fn sync(&mut self, cx: &mut Cx, view: &OptionsView) -> std::io::Result<()> {
        for (r, row) in self.rows.iter_mut().zip(&view.rows) {
            Self::paint_switch(&self.ink, cx, r, row.on);
        }
        cx.follow(ids::option(view.focus.min(self.rows.len().saturating_sub(1))));
        Ok(())
    }

    pub fn hover(&mut self, cx: &mut Cx, id: Id, on: bool) {
        if self.back.id == id {
            self.back.set_hover(cx.kit, on && cx.engine.focus.current() != Some(id));
        }
    }

    pub fn press(&mut self, cx: &mut Cx, id: Id, down: bool) {
        if self.back.id == id {
            self.back.set_pressed(cx.kit, down);
        }
    }

    pub fn target(&self, id: Id) -> Option<Target> {
        if id == ids::OPTIONS_BACK {
            return Some(Target::Back);
        }
        self.rows.iter().position(|r| r.id == id).map(Target::Option)
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn layers(&self) -> usize {
        self.parts.layers() + self.back.layers() + self.rows.iter().map(|r| 3 + r._keep.len() + usize::from(r.tick.is_some())).sum::<usize>()
    }

    /// Gives the engine back what the page registered (the rows, the way back, the scope).
    pub fn retire_engine(&mut self, engine: &mut crate::kit::Engine) {
        for r in &self.rows {
            engine.focus.unregister(r.id);
            engine.flow.drop_lift(r.id);
        }
        self.back.detach(engine);
        if self.modal {
            engine.focus.close_modal();
            self.modal = false;
        }
    }

    pub fn retire(&mut self, cx: &mut Cx, ms: u32) {
        self.retire_engine(cx.engine);
        self.back.hide(cx.kit, ms == 0);
        self.parts.fade(cx.kit, false, ms);
    }

    pub fn row_kinds(&self) -> Vec<&'static str> {
        self.rows.iter().map(|r| r.kind).collect()
    }

    pub fn row_on(&self) -> Vec<bool> {
        self.rows.iter().map(|r| r.on).collect()
    }

    pub fn feed_lint(&self, kit: &mut crate::kit::Kit, lint: &mut crate::kit::lint::Lint) {
        let panel = lint.panel("options", self.frame, self.paint, true);
        for w in &self.words {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
        }
        self.back.feed_lint(kit, lint, panel);
        let style = role_style(kit, TypeRole::Body, None, self.ink.ink, Align::Left);
        let line = kit.line(TypeRole::Body);
        for r in &self.rows {
            let name = crate::cameras::words::kind_name(r.kind);
            let f = Frame::new(r.frame.x + 16.0 + 56.0 + 20.0, r.frame.y + ((r.frame.h - line) / 2.0).round(), width_of(kit, style, name), line);
            lint.word(kit, &format!("option {name}"), name, f, style, Some(panel));
        }
    }

    pub fn paint(&self) -> Rgba {
        self.paint
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::data::Settings;
    use crate::cameras::options::Options;
    use crate::cameras::surface::pic::PictureStore;
    use crate::kit::lint::Lint;
    use crate::kit::testkit;

    fn cx<'a>(kit: &'a mut crate::kit::Kit, engine: &'a mut crate::kit::Engine, store: &'a PictureStore) -> Cx<'a> {
        Cx { kit, engine, store, z: 2000, now_ms: 1000, now_ns: 1_000_000_000 }
    }

    fn options() -> Options {
        Options::new("273", "Front Door", &Settings::default()).unwrap()
    }

    #[test]
    fn the_page_has_a_row_for_each_kind_with_the_defaults_on_and_a_way_back() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let o = options();
        let u = OptionsUi::build(&mut cx(&mut kit, &mut engine, &store), &o.view()).expect("a page");
        assert_eq!(u.row_kinds(), ["person", "vehicle", "animal", "package", "doorbell", "motion"]);
        assert_eq!(u.row_on(), [true, false, false, false, true, false]);
        assert_eq!(engine.focus.modal(), Some(SCOPE_OPTIONS));
        assert_eq!(engine.focus.current(), Some(ids::option(0)));
        assert_eq!(u.target(ids::OPTIONS_BACK), Some(Target::Back));
        assert_eq!(u.target(ids::option(4)), Some(Target::Option(4)));
        let f = u.frame();
        assert_eq!(f.w, 960.0);
        assert!(f.y >= 54.0 && f.bottom() <= 1026.0, "{f:?}");
    }

    #[test]
    fn a_switch_flips_in_place_and_the_focus_follows_the_model() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut o = options();
        let mut u = OptionsUi::build(&mut cx(&mut kit, &mut engine, &store), &o.view()).unwrap();
        o.step(true);
        o.step(true);
        o.toggle(2);
        u.sync(&mut cx(&mut kit, &mut engine, &store), &o.view()).unwrap();
        assert_eq!(u.row_on()[2], true);
        assert_eq!(engine.focus.current(), Some(ids::option(2)));
    }

    #[test]
    fn the_page_reads_clean_to_the_lint_at_both_text_sizes_and_in_both_looks() {
        for scale in [1.0f32, 2.0] {
            for mode in [ui::lucent::palette::Mode::Dark, ui::lucent::palette::Mode::Light] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = crate::kit::Engine::new(&kit).unwrap();
                let store = PictureStore::new();
                let o = options();
                let mut u = OptionsUi::build(&mut cx(&mut kit, &mut engine, &store), &o.view()).unwrap();
                u.sync(&mut cx(&mut kit, &mut engine, &store), &o.view()).unwrap();
                let mut lint = Lint::new().with_safe(Frame::new(64.0, 54.0, 1792.0, 972.0));
                u.feed_lint(&mut kit, &mut lint);
                let findings: Vec<String> = lint.findings().iter().map(|x| x.to_string()).collect();
                assert!(findings.is_empty(), "at {scale} in {mode:?}: {findings:?}");
            }
        }
    }
}
