//! A button of the TV (`kit/LuButton.qml` of the QML shell): a pill with a label and, if it has one, an icon before it. The plain
//! one is the glass wash; the primary one is the accent with the ink that reads on it (one primary button to a task); the danger one
//! is the danger colour on a wash, for the answer that destroys something. It is a focusable of the kit: it registers with the
//! [`Engine`], and its lift (the scale of `focus.buttonScale` and the lit glass, the raised wash and its edge) is the flowing focus's
//! like every other focusable's, handing over between buttons and cards alike. The pointer's hover is a cue of its own (the hover
//! wash and a rim, no lift), pressed shrinks the pill for the press beat, a disabled one dims.
//!
//! The parts hang under one group made at the button's place (the lift's pivot is its centre); `Button` keeps the layers, the
//! engine keeps the group. [`Button::destroy`] gives both back.

use super::flow::Lifted;
use super::focus::{Id, Region, Scope};
use super::icon;
use super::motion::Beat;
use super::text::{Text, role_style, width_of};
use super::{Engine, Kit};
use compositor_protocol::{Ease, Fill, GroupPose, Props, Rgba, Shape};
use lumatv_tokens::TypeRole;
use ui::components::Align;
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Primary,
    Danger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub label: String,
    /// A Lucide icon before the label.
    pub icon: Option<&'static str>,
    pub tone: Tone,
    pub enabled: bool,
}

impl Spec {
    pub fn new(label: &str, tone: Tone) -> Self {
        Self { label: label.to_string(), icon: None, tone, enabled: true }
    }

    pub fn with_icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// The colour `c` (premultiplied) at `a` of its alpha.
pub fn faded(c: Rgba, a: f32) -> Rgba {
    let k = a.clamp(0.0, 1.0);
    let s = |v: u8| (f32::from(v) * k).round() as u8;
    Rgba(s(c.0), s(c.1), s(c.2), s(c.3))
}

/// `c` lightened by `by` (Qt's `lighter`: the channels scaled, clamped), opaque colours only.
fn lighter(c: Rgba, by: f32) -> Rgba {
    let s = |v: u8| (f32::from(v) * by).round().min(255.0) as u8;
    Rgba(s(c.0), s(c.1), s(c.2), c.3)
}

fn pill(fill: Rgba, border: Rgba, radius: f32) -> Shape {
    Shape { corner_radius: radius, fill: Fill::Solid(fill), border_width: 1.0, border }
}

const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

/// Where a button is, as the focus engine knows it: its name, its rectangle on the screen, the region and the scope it focuses in, and
/// its depth.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub id: Id,
    pub frame: Frame,
    pub region: Region,
    pub scope: Scope,
    pub z: i32,
}

pub struct Button {
    pub id: Id,
    frame: Frame,
    spec: Spec,
    base: ShapeLayer,
    lit: ShapeLayer,
    hover: ShapeLayer,
    label: Text,
    icon: Option<ImageLayer>,
    /// The label's and the icon's places inside the button.
    label_at: Props,
    icon_at: Props,
    shown: bool,
    hovered: bool,
}

impl Button {
    /// The height of a button at the person's text size: the target, or the label's line with the padding where the type is large.
    pub fn height(kit: &Kit) -> f32 {
        kit.tokens.target.max(kit.line(TypeRole::Label) + 2.0 * kit.space(2)).ceil()
    }

    fn icon_side(kit: &Kit) -> u32 {
        (22.0 * kit.text_scale).round() as u32
    }

    /// The width of a button of this label: the words, the icon and its gap, and `space(7)` at both ends (an even width).
    pub fn width_for(kit: &mut Kit, label: &str, has_icon: bool) -> f32 {
        let style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Centre);
        let glyph = if has_icon { Self::icon_side(kit) as f32 + kit.space(2) } else { 0.0 };
        let w = (width_of(kit, style, label) + glyph + 2.0 * kit.space(7)).ceil();
        w + w % 2.0
    }

    /// Makes the button at `place.frame` (its size is the frame's), registers it with the engine in the place's region and scope, and
    /// gives it its lift. Nothing shows until [`Button::show`].
    pub fn new(kit: &mut Kit, engine: &mut Engine, place: Place, spec: Spec) -> std::io::Result<Self> {
        let Place { id, frame, region, scope, z } = place;
        let (w, h) = (frame.w, frame.h);
        let radius = (h / 2.0).floor();
        let look = &kit.ui.look;
        let (glass, edge, raised, edge_raised) =
            (look.colour(PaletteRole::Glass), look.colour(PaletteRole::Edge), look.colour(PaletteRole::GlassRaised), look.colour(PaletteRole::EdgeRaised));
        let (accent, danger, ink) = (look.accent(), look.danger(), look.ink());
        let (base_shape, lit_shape, label_colour) = match spec.tone {
            Tone::Plain => (pill(glass, edge, radius), pill(raised, edge_raised, radius), ink),
            Tone::Primary => (pill(accent, edge, radius), pill(lighter(accent, 1.12), edge_raised, radius), look.accent_ink()),
            Tone::Danger => (pill(faded(danger, 0.16), faded(danger, 0.5), radius), pill(faded(danger, 0.24), edge_raised, radius), danger),
        };
        let hover_fill = faded(ink, kit.tokens.states.hover_wash);
        let hover_edge = faded(edge_raised, 0.6);
        let icon_name = spec.icon.filter(|n| kit.icons.has(n));
        let side = Self::icon_side(kit);
        let style = role_style(kit, TypeRole::Label, None, label_colour, Align::Centre);
        let label_w = width_of(kit, style, &spec.label);
        let glyph = if icon_name.is_some() { side as f32 + kit.space(2) } else { 0.0 };
        let line = kit.line(TypeRole::Label);
        let label_box = Frame::new(glyph, ((h - line) / 2.0).round(), w - glyph, line);
        let icon_at = Props::at(((w - label_w - glyph) / 2.0).round(), ((h - side as f32) / 2.0).round());

        let rest = GroupPose { x: frame.x, y: frame.y, pivot_x: w / 2.0, pivot_y: h / 2.0, ..GroupPose::IDENTITY };
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let group = kit.ui.scene.group(None, rest).map_err(other)?;
        let lit_group = kit.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let scene = kit.ui.scene.clone();
        let (base, hover, label, icon) = scene.adopting(&group, || -> std::io::Result<_> {
            let base = kit.ui.scene.shape(w.round() as u32, h.round() as u32, z, base_shape, HIDDEN);
            let hover = kit.ui.scene.shape(w.round() as u32, h.round() as u32, z + 2, pill(hover_fill, hover_edge, radius), HIDDEN);
            let mut label = Text::new(kit, label_box, z + 3, style, &spec.label)?;
            label.set_props(HIDDEN);
            let icon = match icon_name {
                Some(n) => icon::layer(kit, n, side, label_colour, z + 3, HIDDEN)?,
                None => None,
            };
            Ok((base, hover, label, icon))
        })?;
        let lit = scene.adopting(&lit_group, || kit.ui.scene.shape(w.round() as u32, h.round() as u32, z + 1, lit_shape, HIDDEN));
        engine.focus.register(id, frame, region, scope);
        engine.focus.set_enabled(id, spec.enabled);
        engine.flow.lift(id, Lifted { group, rest, scale: kit.tokens.focus.button_scale, rise: 0.0, lit: Some(lit_group) });
        Ok(Self { id, frame, spec, base, lit, hover, label, icon, label_at: Props::at(label_box.x, label_box.y), icon_at, shown: false, hovered: false })
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// The layers this button holds (the scene's budget): the pill, the lit pill, the hover, the label and the icon.
    pub fn layers(&self) -> usize {
        4 + usize::from(self.icon.is_some())
    }

    fn opacity(&self, kit: &Kit) -> f32 {
        if self.spec.enabled { 1.0 } else { kit.tokens.states.disabled_opacity }
    }

    /// Shows the button (fading in over the layer beat, or at once).
    pub fn show(&mut self, kit: &Kit, at_once: bool) {
        let o = self.opacity(kit);
        let at = |p: Props| Props { opacity: o, ..p };
        kit.arrive(&self.base, at(Props::at(0.0, 0.0)), Beat::Layer, at_once);
        kit.arrive(self.label.layer(), at(self.label_at), Beat::Layer, at_once);
        if let Some(i) = &self.icon {
            kit.arrive(i, at(self.icon_at), Beat::Layer, at_once);
        }
        // the lit pill and the hover cue are visible always: their opacity is the lift's and the cue's
        self.lit.set(Props::at(0.0, 0.0));
        self.hover.set(Props { opacity: if self.hovered { 1.0 } else { 0.0 }, visible: true, ..Props::at(0.0, 0.0) });
        self.shown = true;
    }

    /// Takes the button away (fading out, or at once).
    pub fn hide(&mut self, kit: &Kit, at_once: bool) {
        let o = self.opacity(kit);
        let from = |p: Props| Props { opacity: o, ..p };
        if at_once {
            for l in [&*self.base, self.label.layer(), &*self.lit, &*self.hover] {
                l.set(HIDDEN);
            }
            if let Some(i) = &self.icon {
                i.set(HIDDEN);
            }
        } else {
            kit.leave(&self.base, from(Props::at(0.0, 0.0)), Beat::Layer);
            kit.leave(self.label.layer(), from(self.label_at), Beat::Layer);
            if let Some(i) = &self.icon {
                kit.leave(i, from(self.icon_at), Beat::Layer);
            }
            self.lit.set(HIDDEN);
            self.hover.set(HIDDEN);
        }
        self.shown = false;
    }

    /// The hover cue on or off (the pointer is over the button and it is not the focus).
    pub fn set_hover(&mut self, kit: &Kit, on: bool) {
        if self.hovered == on {
            return;
        }
        self.hovered = on;
        if self.shown {
            kit.glide(&self.hover, Props { opacity: if on { 1.0 } else { 0.0 }, ..Props::at(0.0, 0.0) }, Beat::Press);
        }
    }

    /// The pointer is down on the button (the press scale) or let go.
    pub fn set_pressed(&mut self, kit: &Kit, on: bool) {
        if !self.shown {
            return;
        }
        let scale = if on { kit.tokens.focus.press_scale } else { 1.0 };
        let o = self.opacity(kit);
        let ms = kit.beat_ms(Beat::Press);
        for (layer, at) in [(&*self.base, Props::at(0.0, 0.0)), (self.label.layer(), self.label_at)] {
            let to = Props { scale, opacity: o, ..at };
            if ms == 0 { layer.set(to) } else { layer.animate(to, ms, Ease::Out) }
        }
    }

    /// Enables or disables the button: a disabled one dims and takes no focus.
    pub fn set_enabled(&mut self, kit: &Kit, engine: &mut Engine, enabled: bool) {
        if self.spec.enabled == enabled {
            return;
        }
        self.spec.enabled = enabled;
        engine.focus.set_enabled(self.id, enabled);
        if self.shown {
            let o = self.opacity(kit);
            self.base.set(Props { opacity: o, ..Props::at(0.0, 0.0) });
            self.label.set_props(Props { opacity: o, ..self.label_at });
            if let Some(i) = &self.icon {
                i.set(Props { opacity: o, ..self.icon_at });
            }
        }
    }

    /// New words on the button (the box is as wide as it was made: the words are centred in it).
    pub fn set_label(&mut self, kit: &mut Kit, text: &str) {
        self.spec.label = text.to_string();
        self.label.set(kit, text);
    }

    /// Gives the button back: it leaves the engine's registry and loses its lift; its layers go with it.
    pub fn destroy(self, engine: &mut Engine) {
        engine.focus.unregister(self.id);
        engine.flow.drop_lift(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    #[test]
    fn a_button_registers_with_the_engine_lifts_and_hides_until_shown() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).expect("engine");
        let w = Button::width_for(&mut kit, "Switch to HDMI 1", false);
        assert!(w > 200.0 && w % 2.0 == 0.0, "{w}");
        let h = Button::height(&kit);
        assert_eq!(h, 64.0, "the target at 100 % text");
        let before = kit.room();
        let place = Place { id: Id(7), frame: Frame::new(100.0, 100.0, w, h), region: Region(0), scope: Scope::BASE, z: 10 };
        let mut b = Button::new(&mut kit, &mut engine, place, Spec::new("Switch to HDMI 1", Tone::Primary).with_icon("check")).expect("a button");
        let after = kit.room();
        assert_eq!(before.layers - after.layers, b.layers(), "the layers it says it holds are the layers it made");
        assert!(!b.is_shown());
        assert!(engine.focus.focus(Id(7)), "it is a live focusable");
        assert!(engine.focus.is_live(Id(7)));
        b.show(&kit, true);
        assert!(b.is_shown());
        b.set_enabled(&kit, &mut engine, false);
        assert!(!engine.focus.is_live(Id(7)), "a disabled button takes no focus");
        b.destroy(&mut engine);
        assert!(engine.focus.frame(Id(7)).is_none());
    }

    #[test]
    fn the_width_follows_the_words_and_the_height_follows_large_text() {
        let (_h, mut kit) = testkit::kit(1.0);
        let short = Button::width_for(&mut kit, "OK", false);
        let long = Button::width_for(&mut kit, "Turn off every light", false);
        assert!(long > short + 100.0);
        assert!(Button::width_for(&mut kit, "OK", true) > short, "the icon takes room");
        let (_h2, kit2) = testkit::kit(2.0);
        assert!(Button::height(&kit2) > 64.0, "at 200 % the label's line is taller than the target: {}", Button::height(&kit2));
    }
}
