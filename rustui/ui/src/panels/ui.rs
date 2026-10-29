//! What the panels' surfaces are made of beyond the kit: the root a panel's layers hang under and how it arrives and leaves (a slide on a
//! motion track of the compositor, so a frame of it costs this process nothing), a list that scrolls on the focus's spring behind a window,
//! words of several lines in one layer, the glyph hints' widths, and the notes a surface keeps for the lint.
//!
//! A panel is built when it opens and dropped when it has gone (the scene's budget rule); a build while the panel is parked is made inside an
//! inert group, so the compositor takes the pictures between frames and the frame that shows it draws it, with nothing made.

use crate::kit::hint::{Hint, glyph_side};
use crate::kit::lint::Lint;
use crate::kit::text::{Text, role_style};
use crate::kit::viewport::Reach;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Ease, GroupPose, PoseBinding, PoseProperties, Props, Rgba};
use lumatv_tokens::TypeRole;
use ui::components::{Align, Block, Style};
use ui::geometry::Frame;
use ui::group::Group;
use ui::motion::{Motion, MotionBounds, MotionToken, Settle, Units};
use ui::scene::ShapeLayer;
use ui::text::Weight;

pub const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

fn other(e: ui::error::UiError) -> std::io::Error {
    std::io::Error::other(format!("{e}"))
}

/// How a panel arrives: from the right edge, up from the bottom edge, or as a fade (reduced motion, and what has no edge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arrive {
    Right(f32),
    Bottom(f32),
    Fade,
    /// Out of a rectangle (a card's) into another: the whole panel grows from the place and size of `from` to `to` (both `[x, y, w, h]`) as it
    /// fades in, uniformly (its width against the card's width). Reduced motion makes it a fade.
    Grow { from: [f32; 4], to: [f32; 4] },
    /// Lucent's container transform: the panel is the container, grown out of the rectangle `from` (the control that opened it) into `to`
    /// and back into it when it leaves. The container is solid within the first fifth of the way, and the content a [`Root::content`] makes
    /// comes in as it settles. Reduced motion makes it a fade.
    Container { from: [f32; 4], to: [f32; 4] },
}

/// The group a panel's layers hang under. Its pose follows a motion track from 0 (away) to 1 (rest); the track is the compositor's.
pub struct Root {
    pub group: Group,
    track: Motion,
    shown: bool,
    /// It arrives as a container transform: its content fades in late, where an ordinary root's fades with it.
    container: bool,
}

impl Root {
    /// A root that arrives as `arrive` says (a fade under reduced motion); `at_rest` makes it as it is when shown (a rebuild of what is up).
    /// It is inert (not drawn) until [`Root::show`], or at once when `at_rest`.
    pub fn new(kit: &Kit, arrive: Arrive, at_rest: bool) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let arrive = if kit.ui.look.reduced_motion { Arrive::Fade } else { arrive };
        // A binding drives translation, scale and opacity, not the pivot: the group is made with the pivot a grow scales about (the middle
        // of the rectangle it grows into), or the scale would be about the screen's corner and the panel would drift as it grows.
        let pivot = match arrive {
            Arrive::Grow { to, .. } | Arrive::Container { to, .. } => GroupPose { pivot_x: to[0] + to[2] / 2.0, pivot_y: to[1] + to[3] / 2.0, ..GroupPose::IDENTITY },
            _ => GroupPose::IDENTITY,
        };
        let group = scene.group(None, pivot).map_err(other)?;
        let track = scene.motion(if at_rest { 1.0 } else { 0.0 }, MotionBounds::range(0.0, 1.0, Units::Progress)).map_err(other)?;
        let container = matches!(arrive, Arrive::Container { .. });
        let (from, to, properties) = match arrive {
            Arrive::Right(travel) => (GroupPose { x: travel, ..GroupPose::IDENTITY }, GroupPose::IDENTITY, PoseProperties::TRANSLATION),
            Arrive::Bottom(rise) => (GroupPose { y: rise, ..GroupPose::IDENTITY }, GroupPose::IDENTITY, PoseProperties::TRANSLATION),
            Arrive::Fade => (GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }, GroupPose::IDENTITY, PoseProperties::OPACITY),
            Arrive::Grow { from, to } | Arrive::Container { from, to } => {
                let (cf, ct) = ((from[0] + from[2] / 2.0, from[1] + from[3] / 2.0), (to[0] + to[2] / 2.0, to[1] + to[3] / 2.0));
                let scale = (from[2] / to[2].max(1.0)).clamp(0.05, 1.0);
                let opacity = if container { 1.0 } else { 0.0 };
                (GroupPose { x: cf.0 - ct.0, y: cf.1 - ct.1, scale, pivot_x: ct.0, pivot_y: ct.1, opacity, ..GroupPose::IDENTITY }, GroupPose::IDENTITY, if container { PoseProperties::TRANSLATION.union(PoseProperties::SCALE) } else { PoseProperties::ALL })
            }
        };
        group.bind(&track, PoseBinding { values: [0.0, 1.0], from, to, properties }).map_err(other)?;
        if container {
            // solid within the first fifth of the way, so what grows out of the bell is the panel's glass and not a faint shape
            let hidden = GroupPose { opacity: 0.0, ..GroupPose::IDENTITY };
            group.bind(&track, PoseBinding { values: [0.0, 0.2], from: hidden, to: GroupPose::IDENTITY, properties: PoseProperties::OPACITY }).map_err(other)?;
        }
        group.set_inert(!at_rest);
        Ok(Self { group, track, shown: at_rest, container })
    }

    /// The group a panel's content hangs under, inside the root. Under a container transform it fades in over the last half of the way
    /// (and out over the first half of the way back); under any other root it is the root's own fade.
    pub fn content(&self, kit: &Kit) -> std::io::Result<Group> {
        let scene = kit.ui.scene.clone();
        let content = scene.group(Some(self.group.id()), GroupPose::IDENTITY).map_err(other)?;
        if self.container {
            let hidden = GroupPose { opacity: 0.0, ..GroupPose::IDENTITY };
            content.bind(&self.track, PoseBinding { values: [0.5, 0.95], from: hidden, to: GroupPose::IDENTITY, properties: PoseProperties::OPACITY }).map_err(other)?;
        }
        Ok(content)
    }

    /// Makes `group` (the dim behind the panel, which does not move with it) follow the root's way in and out as a fade: it is clear at
    /// `values[0]` of the way and solid at `values[1]`. Only under a container transform; another root's dim is not faded.
    pub fn bind_fade(&self, group: &Group, values: [f32; 2]) -> std::io::Result<()> {
        if !self.container {
            return Ok(());
        }
        let hidden = GroupPose { opacity: 0.0, ..GroupPose::IDENTITY };
        group.bind(&self.track, PoseBinding { values, from: hidden, to: GroupPose::IDENTITY, properties: PoseProperties::OPACITY }).map_err(other)
    }

    /// Shows the panel on `settle` (the shared spring: retargeted from wherever it is and however fast it is going).
    pub fn show_with(&mut self, settle: Settle) {
        self.group.set_inert(false);
        let _ = self.track.settle(Some(1.0), None, settle);
        self.shown = true;
    }

    /// Takes the panel away on `settle`.
    pub fn hide_with(&mut self, settle: Settle) {
        let _ = self.track.settle(Some(0.0), None, settle);
        self.shown = false;
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// Whether it arrives as a container transform (not under reduced motion, which is a fade).
    pub fn is_container(&self) -> bool {
        self.container
    }

    fn settle(&self, to: f32, ms: u32, ease: Ease) {
        let _ = self.track.settle(Some(to), None, Settle::Timed(MotionToken { duration_ms: ms, ease }));
    }

    /// Shows the panel: it is drawn from the next frame and arrives over `ms`.
    pub fn show(&mut self, ms: u32) {
        self.group.set_inert(false);
        self.settle(1.0, ms, Ease::Out);
        self.shown = true;
    }

    /// Takes the panel away over `ms`; it is parked (not drawn) once it has gone.
    pub fn hide(&mut self, ms: u32) {
        self.settle(0.0, ms, Ease::In);
        self.shown = false;
    }

    /// Not drawn at all (the slide out is over).
    pub fn park(&mut self) {
        self.group.set_inert(true);
        self.shown = false;
    }
}

/// A full-screen dim behind a panel: the theme's scrim colour, hidden until shown.
pub fn scrim(kit: &Kit, z: i32, hide: bool) -> ShapeLayer {
    let fill = if hide { kit.tokens.extra(kit.mode).scrim_hide } else { kit.ui.look.colour(ui::lucent::palette::PaletteRole::Scrim) };
    let layer = ui::components::plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, ui::components::plate::solid(fill, 0.0));
    layer.set(HIDDEN);
    layer
}

pub fn scrim_set(layer: &ShapeLayer, on: bool) {
    layer.set(if on { Props::at(0.0, 0.0) } else { HIDDEN });
}

/// A picture file shown whole inside `w` by `h` (an app's own icon), as a layer at `at`; none when the file is not a picture.
pub fn picture_layer(kit: &mut Kit, file: &str, w: u32, h: u32, z: i32, at: Props) -> std::io::Result<Option<ui::scene::ImageLayer>> {
    if file.is_empty() {
        return Ok(None);
    }
    let Some(p) = crate::kit::avatar::contained(file, w, h) else { return Ok(None) };
    let mut layer = kit.ui.scene.image(p.width, p.height, z, at)?;
    p.paint(&mut layer);
    Ok(Some(layer))
}

/// A window onto content taller than itself, moved by whole rows on the focus's spring: `inner` holds the content in screen coordinates at
/// offset 0, the window clips it, and the offset is a motion track of the compositor bound to `inner`'s translation.
pub struct Scroll {
    pub inner: Group,
    _clip: Group,
    track: Option<Motion>,
    window: Frame,
    extent: f32,
    offset: f32,
}

impl Scroll {
    /// `window` is where the content shows, `content_h` how tall the content is from the window's top. The clip is the window grown by
    /// `reach`, what the focusables in it stand out by when lifted ([`Reach::lift`]), so the focused plate is not cut.
    pub fn new(kit: &Kit, parent: &Group, window: Frame, content_h: f32, reach: Reach) -> std::io::Result<Self> {
        Self::make(kit, parent, window, content_h, false, reach)
    }

    /// A window onto content wider than itself (a row of cards): `content_w` is how wide the content is from the window's left edge.
    pub fn row(kit: &Kit, parent: &Group, window: Frame, content_w: f32, reach: Reach) -> std::io::Result<Self> {
        Self::make(kit, parent, window, content_w, true, reach)
    }

    fn make(kit: &Kit, parent: &Group, window: Frame, content: f32, across: bool, reach: Reach) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let clip = scene.group(Some(parent.id()), GroupPose { clip: Some(reach.clip(window)), ..GroupPose::IDENTITY }).map_err(other)?;
        let inner = scene.group(Some(clip.id()), GroupPose::IDENTITY).map_err(other)?;
        let extent = (content - if across { window.w } else { window.h }).max(0.0);
        let track = if extent > 0.0 {
            let t = scene.motion(0.0, MotionBounds::range(0.0, extent, Units::Pixels)).map_err(other)?;
            let to = if across { GroupPose { x: -extent, ..GroupPose::IDENTITY } } else { GroupPose { y: -extent, ..GroupPose::IDENTITY } };
            inner.bind(&t, PoseBinding { values: [0.0, extent], from: GroupPose::IDENTITY, to, properties: PoseProperties::TRANSLATION }).map_err(other)?;
            Some(t)
        } else {
            None
        };
        Ok(Self { inner, _clip: clip, track, window, extent, offset: 0.0 })
    }

    pub fn window(&self) -> Frame {
        self.window
    }

    pub fn extent(&self) -> f32 {
        self.extent
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// Scrolls to `offset` (clamped) on the focus spring; `at_once` jumps.
    pub fn to(&mut self, engine: &Engine, offset: f32, at_once: bool) {
        let offset = offset.clamp(0.0, self.extent);
        self.offset = offset;
        if let Some(t) = &self.track {
            let settle = if at_once { Settle::Timed(MotionToken { duration_ms: 0, ease: Ease::Linear }) } else { engine.flow.spec.settle() };
            let _ = t.settle(Some(offset), if at_once { Some(0.0) } else { None }, settle);
        }
    }

    pub fn settling_at(&self, at_ns: u64) -> bool {
        self.track.as_ref().is_some_and(|t| t.is_settling_at(at_ns))
    }

    /// Whether a row of `top` and `height` (in content coordinates from the window's top) lies whole inside the window at `offset`.
    pub fn fits(&self, top: f32, height: f32, offset: f32) -> bool {
        top >= offset - 0.5 && top + height <= offset + self.window.h + 0.5
    }

    /// The offset that shows a row of `top` and `height` whole, moving as little as it can: unchanged when it already shows, else up to its
    /// top, else down to the earliest row top from which it ends inside the window.
    pub fn reveal(&self, top: f32, height: f32, tops: &[f32]) -> f32 {
        let cur = self.offset;
        if self.fits(top, height, cur) {
            return cur;
        }
        if top < cur {
            return top;
        }
        let mut o = top;
        for t in tops.iter().rev().filter(|t| **t < top) {
            if top + height - *t > self.window.h + 0.5 {
                break;
            }
            o = *t;
        }
        o
    }
}

/// One layer of several lines of words, each in its role, ink and weight, stacked from the top.
pub struct Words {
    block: Option<Block>,
    frame: Frame,
    lines: Vec<(String, Style)>,
}

/// A line of a [`Words`]: its text, role, weight and ink.
pub struct WordLine {
    pub text: String,
    pub role: TypeRole,
    pub weight: Option<Weight>,
    pub ink: Rgba,
    pub align: Align,
}

impl WordLine {
    pub fn new(text: &str, role: TypeRole, ink: Rgba) -> Self {
        Self { text: text.to_string(), role, weight: None, ink, align: Align::Left }
    }

    pub fn aligned(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn weight(mut self, w: Weight) -> Self {
        self.weight = Some(w);
        self
    }
}

impl Words {
    /// The height `lines` take, one under the other.
    pub fn height_of(kit: &Kit, lines: &[WordLine]) -> f32 {
        lines.iter().map(|l| kit.line(l.role)).sum()
    }

    /// The words at (`x`, `y`) in a box `w` wide, at depth `z`; an empty list makes no layer.
    pub fn new(kit: &mut Kit, x: f32, y: f32, w: f32, z: i32, lines: &[WordLine]) -> std::io::Result<Self> {
        let h = Self::height_of(kit, lines);
        let mut frames = Vec::new();
        let mut top = 0.0;
        let mut kept = Vec::new();
        for l in lines {
            let style = role_style(kit, l.role, l.weight, l.ink, l.align);
            let lh = kit.line(l.role);
            frames.push((Frame::new(0.0, top, w, lh), style, l.text.clone()));
            kept.push((l.text.clone(), style));
            top += lh;
        }
        let frame = Frame::new(x, y, w, h);
        let block = if lines.is_empty() || w < 1.0 { None } else { Some(Block::new(&mut kit.ui, frame, z, frames)?) };
        Ok(Self { block, frame, lines: kept })
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn text(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", |l| l.0.as_str())
    }

    pub fn set(&mut self, kit: &mut Kit, i: usize, text: &str) {
        if let (Some(b), Some(l)) = (self.block.as_mut(), self.lines.get_mut(i))
            && l.0 != text
        {
            b.set(&mut kit.ui, i, text);
            l.0 = text.to_string();
        }
    }

    pub fn layer(&self) -> Option<&ui::scene::Layer> {
        self.block.as_ref().map(|b| b.layer())
    }

    pub fn layers(&self) -> usize {
        usize::from(self.block.is_some())
    }

    /// Describes the lines to a lint, each as the word it is on the screen (the box shifted by `shift`, a scrolled list's offset); the
    /// layer the compositor places is the block's, at the box's origin.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint, name: &str, panel: Option<usize>, shift: (f32, f32)) {
        let layer = (self.frame.x + shift.0, self.frame.y + shift.1);
        let mut top = 0.0;
        for (i, (text, style)) in self.lines.iter().enumerate() {
            let lh = ui::components::line_height(style.ty);
            let f = Frame::new(layer.0, layer.1 + top, self.frame.w, lh);
            lint.word_on(kit, &format!("{name} line {i}"), text, f, *style, panel, layer);
            top += lh;
        }
    }
}

/// A [`Text`] that is one line in a box, with the style it was made with (for the lint).
pub struct Line {
    pub text: Text,
    pub style: Style,
}

impl Line {
    pub fn new(kit: &mut Kit, frame: Frame, z: i32, style: Style, text: &str) -> std::io::Result<Self> {
        Ok(Self { text: Text::new(kit, frame, z, style, text)?, style })
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint, name: &str, panel: Option<usize>, shift: (f32, f32)) {
        let f = self.text.frame();
        lint.word(kit, name, self.text.text(), Frame::new(f.x + shift.0, f.y + shift.1, f.w, f.h), self.style, panel);
    }
}

/// The width a hint takes drawn in a row beside text of `role` (`HintRow`'s own arithmetic: the glyphs at 1.45 times the role's size with
/// the gap between them, then the label).
pub fn hint_width(kit: &mut Kit, hint: &Hint, role: TypeRole) -> f32 {
    let side = glyph_side(kit, role) as f32;
    let gap = kit.space(2);
    let n = hint.glyphs().len() as f32;
    let mut w = n * side + (n - 1.0).max(0.0) * gap;
    if let Some(label) = &hint.label {
        let style = role_style(kit, role, None, kit.ui.look.ink(), Align::Left);
        w += gap + crate::kit::text::width_of(kit, style, label);
    }
    w
}

/// The width of a row of hints one after the other with `gap` between them.
pub fn hints_width(kit: &mut Kit, hints: &[Hint], role: TypeRole, gap: f32) -> f32 {
    let widths: Vec<f32> = hints.iter().map(|h| hint_width(kit, h, role)).collect();
    widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1)) as f32
}

/// The engine's focus follows the model's: the move flows from where the focus was.
pub fn follow(engine: &mut Engine, want: Option<crate::kit::Id>, now_ns: u64) {
    match want {
        Some(id) if engine.focus.current() != Some(id) => {
            engine.focus(id, now_ns);
        }
        None if engine.focus.current().is_some() => engine.clear(now_ns),
        _ => {}
    }
}

/// How many layers a panel may hold at most: what the scene has left, less a margin for the system layer's own.
pub fn budget(kit: &Kit) -> usize {
    kit.room().layers.saturating_sub(16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::kit::widgets::Lift;

    #[test]
    fn a_root_is_inert_until_shown_arrives_on_a_track_and_is_parked_when_it_has_gone() {
        let (_h, kit) = testkit::kit(1.0);
        let tracks = kit.room().tracks;
        let mut r = Root::new(&kit, Arrive::Right(600.0), false).expect("a root");
        assert!(!r.is_shown() && r.group.is_inert());
        assert_eq!(tracks - kit.room().tracks, 1, "one track of the compositor's");
        r.show(110);
        assert!(r.is_shown() && !r.group.is_inert());
        r.hide(110);
        assert!(!r.is_shown() && !r.group.is_inert(), "drawn while it slides out");
        r.park();
        assert!(r.group.is_inert());
        let rest = Root::new(&kit, Arrive::Bottom(400.0), true).expect("a root at rest");
        assert!(rest.is_shown() && !rest.group.is_inert(), "a rebuild of what is up is drawn at once");
    }

    #[test]
    fn a_scroll_has_a_track_only_when_the_content_is_taller_than_the_window_and_reveals_a_row_whole() {
        let (_h, kit) = testkit::kit(1.0);
        let root = kit.ui.scene.group(None, GroupPose::IDENTITY).expect("a group");
        let window = Frame::new(0.0, 100.0, 500.0, 300.0);
        let flat = Scroll::new(&kit, &root, window, 250.0, Reach::NONE).expect("a scroll");
        assert_eq!(flat.extent(), 0.0);
        let tracks = kit.room().tracks;
        let mut s = Scroll::new(&kit, &root, window, 700.0, Reach::NONE).expect("a scroll");
        assert_eq!(tracks - kit.room().tracks, 1);
        assert_eq!(s.extent(), 400.0);
        let tops = [0.0, 120.0, 240.0, 360.0, 480.0, 600.0];
        // rows of 100 at those tops: the fourth (360..460) does not fit a window of 300 at offset 0
        assert_eq!(s.reveal(360.0, 100.0, &tops), 240.0, "down to the earliest row top from which it ends inside: 240 (240..540 holds 360..460)");
        let engine = Engine::new(&kit).expect("an engine");
        s.to(&engine, 240.0, true);
        assert_eq!(s.offset(), 240.0);
        assert_eq!(s.reveal(240.0, 100.0, &tops), 240.0, "a row that shows is not moved for");
        assert_eq!(s.reveal(0.0, 100.0, &tops), 0.0, "up to a row above");
        s.to(&engine, 9999.0, false);
        assert_eq!(s.offset(), 400.0, "clamped");
        assert!(s.fits(480.0, 100.0, 400.0) && !s.fits(240.0, 100.0, 400.0));
    }

    #[test]
    fn a_scroll_clips_to_its_window_and_the_reach_of_what_lifts_in_it_and_a_scrolled_clip_stays_put() {
        let (_h, kit) = testkit::kit(1.0);
        let root = kit.ui.scene.group(None, GroupPose::IDENTITY).expect("a group");
        let window = Frame::new(40.0, 100.0, 500.0, 300.0);
        let reach = Reach::lift(&kit, Lift::Row, (500.0, 70.0));
        let mut s = Scroll::new(&kit, &root, window, 700.0, reach).expect("a scroll");
        let clip = s.inner.world().clip.expect("a clip");
        assert!((clip.x - (40.0 - reach.left)).abs() < 0.01 && (clip.w - (500.0 + reach.left + reach.right)).abs() < 0.01, "{clip:?} for {reach:?}");
        assert!(reach.left > 4.0 && (clip.y - (100.0 - reach.top)).abs() < 0.01, "a row at 1.02 of 500 px leaves its window by 5 px and more: {clip:?}");
        let engine = Engine::new(&kit).expect("an engine");
        s.to(&engine, 240.0, true);
        assert_eq!(s.inner.world().clip, Some(clip), "the clip is the window's, not the content's: it does not scroll");
    }

    #[test]
    fn words_are_one_layer_of_lines_in_their_roles_and_change_in_place() {
        let (_h, mut kit) = testkit::kit(1.0);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let lines = [WordLine::new("Picture mode", TypeRole::Label, ink), WordLine::new("HDMI 1", TypeRole::Caption, ink2)];
        let before = kit.room().layers;
        let mut w = Words::new(&mut kit, 10.0, 20.0, 300.0, 5, &lines).expect("words");
        assert_eq!(before - kit.room().layers, 1, "one layer for both lines");
        assert_eq!(w.frame().h, kit.line(TypeRole::Label) + kit.line(TypeRole::Caption));
        w.set(&mut kit, 1, "HDMI 2");
        assert_eq!(w.text(1), "HDMI 2");
        let mut lint = Lint::new();
        w.feed_lint(&mut kit, &mut lint, "row", None, (0.0, 0.0));
        assert_eq!(lint.words().len(), 2);
        let none = Words::new(&mut kit, 0.0, 0.0, 100.0, 5, &[]).expect("none");
        assert!(none.is_empty() && none.layers() == 0);
    }

    #[test]
    fn a_hints_width_is_its_glyphs_and_the_gaps_between_them() {
        let (_h, mut kit) = testkit::kit(1.0);
        let hint = Hint::key(crate::kit::hint::Key::Ok, crate::kit::hint::Action::Expand);
        let side = glyph_side(&kit, TypeRole::Label) as f32;
        assert_eq!(hint_width(&mut kit, &hint, TypeRole::Label), 2.0 * side + kit.space(2));
        let two = [hint.clone(), hint];
        assert_eq!(hints_width(&mut kit, &two, TypeRole::Label, 20.0), 2.0 * (2.0 * side + kit.space(2)) + 20.0);
    }
}
