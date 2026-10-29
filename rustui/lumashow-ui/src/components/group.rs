//! Layers that move together: a [`Part`] is one compositor layer with the place it rests at, a [`Group`] is the
//! layers of one control, card or page, and a [`Pose`] is how the whole group is displaced, scaled and faded from
//! rest. Everything a screen animates (a press, a page sliding in, an overlay fading) is one `Pose` applied to a
//! group, so it costs one `Animate` request per layer and nothing per frame: the compositor runs the clock.

use super::label::Label;
use super::text_layer::TextLayer;
use crate::geometry::Frame;
use crate::lucent::tokens::state::PRESSED_SCALE;
use crate::scene::{GlassLayer, ImageLayer, LiveLayer, MeshLayer, ShapeLayer, WallpaperLayer};
use compositor_protocol::{Ease, Edges, Props, Rect};

/// A group's displacement from rest. The scale is about the group's `about` point, so a pressed button shrinks
/// towards its own middle and not towards the layers' corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub opacity: f32,
    pub visible: bool,
}

impl Pose {
    pub const REST: Pose = Pose {
        dx: 0.0,
        dy: 0.0,
        scale: 1.0,
        opacity: 1.0,
        visible: true,
    };

    pub const fn at(dx: f32, dy: f32) -> Pose {
        Pose {
            dx,
            dy,
            ..Pose::REST
        }
    }

    pub const fn fade(opacity: f32) -> Pose {
        Pose {
            opacity,
            ..Pose::REST
        }
    }

    pub const fn scaled(scale: f32) -> Pose {
        Pose {
            scale,
            ..Pose::REST
        }
    }

    /// `scale.pressed` (Lucent tokens.json): a touched control settles towards the finger.
    pub const fn pressed() -> Pose {
        Pose::scaled(PRESSED_SCALE)
    }

    /// Offscreen or otherwise not drawn at all (a hidden view is inert, not an opacity-zero ghost).
    pub const fn hidden() -> Pose {
        Pose {
            visible: false,
            ..Pose::REST
        }
    }

    /// `at` and `fade` together: the usual entrance of a sheet, a card or a page.
    pub const fn slid(dx: f32, dy: f32, opacity: f32) -> Pose {
        Pose {
            dx,
            dy,
            opacity,
            ..Pose::REST
        }
    }
}

/// What a layer keeps whatever its group does: its opacity and scale at rest (a dimmed night digit; a small picture
/// the compositor enlarges), and the clip that shows only part of it, with the feather of the clip's edges. Every
/// `Set`/`Animate` must carry them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rest {
    pub opacity: f32,
    pub scale: f32,
    pub clip: Option<Rect>,
    pub feather: Edges,
}

impl Rest {
    pub const PLAIN: Rest = Rest {
        opacity: 1.0,
        scale: 1.0,
        clip: None,
        feather: Edges::NONE,
    };
}

/// The properties of a layer resting at `frame` (logical pixels) when its group is in `pose` about `about`. The
/// compositor scales a layer about its own middle, so the layer's middle is moved to where the scaled group puts
/// it, and the layer's own `rest` scale multiplies the group's.
pub fn pose_props(frame: Frame, rest: Rest, about: (f32, f32), pose: Pose) -> Props {
    let (mx, my) = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
    let (x, y) = (
        about.0 + (mx - about.0) * pose.scale + pose.dx,
        about.1 + (my - about.1) * pose.scale + pose.dy,
    );
    Props {
        x: x - frame.w / 2.0,
        y: y - frame.h / 2.0,
        scale: rest.scale * pose.scale,
        opacity: rest.opacity * pose.opacity,
        clip: rest.clip,
        feather: rest.feather,
        visible: pose.visible,
    }
}

/// Whatever the compositor layer of a part is: the screens keep the typed layer so they can still restyle or
/// redraw it.
pub enum Kind {
    Glass(GlassLayer),
    Shape(ShapeLayer),
    Image(ImageLayer),
    Text(Label),
    /// Several lines in several colours, with their marks: one layer, a picture or a glyph run.
    Lines(TextLayer),
    Mesh(MeshLayer),
    /// A producer's frames (a camera's pictures), drawn by the compositor from the producer's own buffers.
    Live(LiveLayer),
    Wallpaper(WallpaperLayer),
}

macro_rules! kind_from {
    ($($ty:ty => $variant:ident),* $(,)?) => {$(
        impl From<$ty> for Kind {
            fn from(layer: $ty) -> Kind {
                Kind::$variant(layer)
            }
        }
    )*};
}

kind_from!(
    GlassLayer => Glass,
    ShapeLayer => Shape,
    ImageLayer => Image,
    Label => Text,
    TextLayer => Lines,
    MeshLayer => Mesh,
    LiveLayer => Live,
    WallpaperLayer => Wallpaper,
);

/// One layer and the place it rests at.
pub struct Part {
    /// Where the layer rests: top-left and size in logical pixels (its unscaled size).
    pub frame: Frame,
    /// The opacity at rest (a dimmed night digit, a secondary label); a pose multiplies it.
    pub rest_opacity: f32,
    /// The scale at rest, about the layer's middle (a small picture the compositor enlarges); a pose multiplies it.
    pub rest_scale: f32,
    /// Only the part of the layer inside this logical rectangle is drawn (the cells of a scrolling strip).
    pub clip: Option<Rect>,
    /// How far in from each edge of `clip` the layer fades out; none is a hard clip.
    pub feather: Edges,
    pub kind: Kind,
}

impl Part {
    pub fn new(kind: impl Into<Kind>, frame: Frame) -> Self {
        Self {
            frame,
            rest_opacity: 1.0,
            rest_scale: 1.0,
            clip: None,
            feather: Edges::NONE,
            kind: kind.into(),
        }
    }

    /// The layer is drawn `scale` times its size, about its middle.
    pub fn scaled(mut self, scale: f32) -> Self {
        self.rest_scale = scale;
        self
    }

    pub fn clipped(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self.feather = Edges::NONE;
        self
    }

    /// Only the part of the layer inside `clip` is drawn, and it fades out over `edges` towards the clip's edges (a strip or
    /// a list that meets the edge of its window).
    pub fn clip_feathered(mut self, clip: Rect, edges: Edges) -> Self {
        self.clip = Some(clip);
        self.feather = edges;
        self
    }

    pub fn props(&self, about: (f32, f32), pose: Pose) -> Props {
        let rest = Rest {
            opacity: self.rest_opacity,
            scale: self.rest_scale,
            clip: self.clip,
            feather: self.feather,
        };
        pose_props(self.frame, rest, about, pose)
    }

    /// Moves the layer to `pose` at once, cancelling any animation on it.
    pub fn set(&mut self, about: (f32, f32), pose: Pose) {
        let props = self.props(about, pose);
        match &mut self.kind {
            Kind::Glass(l) => l.set(props),
            Kind::Shape(l) => l.set(props),
            Kind::Image(l) => l.set(props),
            Kind::Text(l) => l.set_props(props),
            Kind::Lines(l) => l.set_props(props),
            Kind::Mesh(l) => l.set(props),
            Kind::Live(l) => l.set(props),
            Kind::Wallpaper(l) => l.set(props),
        }
    }

    /// Animates the layer from where it is now to `pose`, on the compositor's clock.
    pub fn animate(&mut self, about: (f32, f32), pose: Pose, ms: u32, ease: Ease) {
        let props = self.props(about, pose);
        match &mut self.kind {
            Kind::Glass(l) => l.animate(props, ms, ease),
            Kind::Shape(l) => l.animate(props, ms, ease),
            Kind::Image(l) => l.animate(props, ms, ease),
            Kind::Text(l) => l.animate_props(props, ms, ease),
            Kind::Lines(l) => l.animate_props(props, ms, ease),
            Kind::Mesh(l) => l.animate(props, ms, ease),
            Kind::Live(l) => l.animate(props, ms, ease),
            Kind::Wallpaper(l) => l.animate(props, ms, ease),
        }
    }

    /// The compositor layer of the part, to hang it under a native group (`ui::Group::attach`).
    pub fn layer(&self) -> &crate::scene::Layer {
        match &self.kind {
            Kind::Glass(l) => l,
            Kind::Shape(l) => l,
            Kind::Image(l) => l,
            Kind::Text(l) => l.layer(),
            Kind::Lines(l) => l.layer(),
            Kind::Mesh(l) => l,
            Kind::Live(l) => l,
            Kind::Wallpaper(l) => l,
        }
    }

    pub fn image_mut(&mut self) -> Option<&mut ImageLayer> {
        match &mut self.kind {
            Kind::Image(l) => Some(l),
            _ => None,
        }
    }

    /// The text layer of several lines, if the part is one.
    pub fn lines_mut(&mut self) -> Option<&mut TextLayer> {
        match &mut self.kind {
            Kind::Lines(l) => Some(l),
            _ => None,
        }
    }

    pub fn glass(&self) -> Option<&GlassLayer> {
        match &self.kind {
            Kind::Glass(l) => Some(l),
            _ => None,
        }
    }

    pub fn shape(&self) -> Option<&ShapeLayer> {
        match &self.kind {
            Kind::Shape(l) => Some(l),
            _ => None,
        }
    }
}

/// The parts of one control, card or page, moved together about one point.
pub struct Group {
    pub parts: Vec<Part>,
    pub about: (f32, f32),
    /// Multiplies the opacity of every pose the group puts its parts in: a list that fades what stands at the edge of its window sets
    /// it on a cell without the cell's controls knowing. 1 draws the parts as posed.
    pub alpha: f32,
}

impl Group {
    pub fn new(about: (f32, f32)) -> Self {
        Self {
            parts: Vec::new(),
            about,
            alpha: 1.0,
        }
    }

    /// `pose` with the group's own `alpha` taken into its opacity.
    fn faded(&self, pose: Pose) -> Pose {
        Pose {
            opacity: pose.opacity * self.alpha,
            ..pose
        }
    }

    /// Adds a part and returns its index.
    pub fn push(&mut self, part: Part) -> usize {
        self.parts.push(part);
        self.parts.len() - 1
    }

    pub fn set(&mut self, pose: Pose) {
        let (about, pose) = (self.about, self.faded(pose));
        for part in &mut self.parts {
            part.set(about, pose);
        }
    }

    pub fn animate(&mut self, pose: Pose, ms: u32, ease: Ease) {
        let (about, pose) = (self.about, self.faded(pose));
        for part in &mut self.parts {
            part.animate(about, pose, ms, ease);
        }
    }

    /// `animate` for only the parts at `indices`: a press that crossfades two layers has no business animating the label.
    pub fn animate_parts(&mut self, indices: &[usize], pose: Pose, ms: u32, ease: Ease) {
        let (about, pose) = (self.about, self.faded(pose));
        for &i in indices {
            self.parts[i].animate(about, pose, ms, ease);
        }
    }

    /// `set` for only the parts at `indices`.
    pub fn set_parts(&mut self, indices: &[usize], pose: Pose) {
        let (about, pose) = (self.about, self.faded(pose));
        for &i in indices {
            self.parts[i].set(about, pose);
        }
    }

    /// The text layer at `index`, if the part is one.
    pub fn label_mut(&mut self, index: usize) -> Option<&mut Label> {
        match &mut self.parts[index].kind {
            Kind::Text(label) => Some(label),
            _ => None,
        }
    }

    /// The shape layer at `index`, if the part is one.
    pub fn shape_at(&self, index: usize) -> Option<&ShapeLayer> {
        self.parts[index].shape()
    }
}

impl Group {
    /// `set` about another point and with some parts kept out of sight: a page that moves a control with it puts its own pose on the
    /// control's layers (a focus ring stays hidden unless the control is focused). The group's own `about` is left as it was.
    pub fn set_in(&mut self, about: (f32, f32), pose: Pose, hidden: &[usize]) {
        let pose = self.faded(pose);
        for (i, part) in self.parts.iter_mut().enumerate() {
            part.set(about, Self::posed(i, pose, hidden));
        }
    }

    /// `animate` with the same rules as [`Group::set_in`].
    pub fn animate_in(
        &mut self,
        about: (f32, f32),
        pose: Pose,
        hidden: &[usize],
        duration_ms: u32,
        ease: Ease,
    ) {
        let pose = self.faded(pose);
        for (i, part) in self.parts.iter_mut().enumerate() {
            part.animate(about, Self::posed(i, pose, hidden), duration_ms, ease);
        }
    }

    fn posed(i: usize, pose: Pose, hidden: &[usize]) -> Pose {
        if hidden.contains(&i) {
            Pose {
                visible: false,
                ..pose
            }
        } else {
            pose
        }
    }
}

/// Something made of layers that a page can carry: a button, a switch, a slider. A page poses everything on it with one pose about
/// one point (a sheet sliding, a dialog scaling in), so a component answers to that point and not to its own middle. The layers
/// keep their own rest opacity, clip and scale, as in [`pose_props`].
pub trait Movable {
    /// Moves everything to `pose` about `about` at once.
    fn set_in(&mut self, about: (f32, f32), pose: Pose);
    /// Moves everything to `pose` about `about` on the compositor's clock.
    fn animate_in(&mut self, about: (f32, f32), pose: Pose, duration_ms: u32, ease: Ease);
}

impl Movable for Group {
    fn set_in(&mut self, about: (f32, f32), pose: Pose) {
        Group::set_in(self, about, pose, &[]);
    }

    fn animate_in(&mut self, about: (f32, f32), pose: Pose, duration_ms: u32, ease: Ease) {
        Group::animate_in(self, about, pose, &[], duration_ms, ease);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::{Request, Rgba};

    const F: Frame = Frame::new(100.0, 200.0, 80.0, 40.0);

    fn middle(p: &Props, f: Frame) -> (f32, f32) {
        (p.x + f.w / 2.0, p.y + f.h / 2.0)
    }

    #[test]
    fn a_layer_at_rest_is_drawn_exactly_where_its_frame_is() {
        let p = pose_props(F, Rest::PLAIN, (0.0, 0.0), Pose::REST);
        assert_eq!((p.x, p.y, p.scale, p.opacity), (100.0, 200.0, 1.0, 1.0));
        assert!(p.visible && p.clip.is_none());
    }

    #[test]
    fn a_pressed_button_shrinks_about_its_own_middle_and_its_layers_stay_concentric() {
        let about = (140.0, 220.0);
        let p = pose_props(F, Rest::PLAIN, about, Pose::pressed());
        let (mx, my) = middle(&p, F);
        assert!((mx - 140.0).abs() < 1e-4 && (my - 220.0).abs() < 1e-4);
        assert_eq!(p.scale, PRESSED_SCALE);
    }

    #[test]
    fn a_layer_off_the_groups_middle_moves_towards_it_in_proportion_to_the_scale() {
        // The group's middle is at x=140; a small label whose own middle is 40 px to the right of it ends up
        // 40 * .97 to the right after the press, not 40.
        let label = Frame::new(160.0, 210.0, 40.0, 20.0);
        let p = pose_props(label, Rest::PLAIN, (140.0, 220.0), Pose::pressed());
        let (mx, _) = middle(&p, label);
        assert!((mx - (140.0 + 40.0 * 0.97)).abs() < 1e-3, "{mx}");
    }

    #[test]
    fn a_pose_multiplies_the_rest_opacity_and_keeps_the_clip_and_offset() {
        let clip = Rect {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
        };
        let rest = Rest {
            opacity: 0.5,
            clip: Some(clip),
            ..Rest::PLAIN
        };
        let p = pose_props(F, rest, (0.0, 0.0), Pose::slid(0.0, 16.0, 0.5));
        assert_eq!((p.x, p.y), (100.0, 216.0));
        assert_eq!(p.opacity, 0.25);
        assert_eq!(p.clip, Some(clip));
    }

    #[test]
    fn a_hidden_pose_is_not_drawn_but_still_has_a_place() {
        let p = pose_props(F, Rest::PLAIN, (0.0, 0.0), Pose::hidden());
        assert!(!p.visible);
        assert_eq!((p.x, p.y), (100.0, 200.0));
    }

    fn plate_part(h: &crate::testing::Harness) -> Part {
        let layer = crate::components::plate::shape(
            &h.ui,
            F,
            1,
            crate::components::plate::solid(Rgba(9, 9, 9, 255), 0.0),
        );
        Part::new(layer, F)
    }

    const WINDOW: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 300.0,
        h: 300.0,
    };

    #[test]
    fn a_feathered_part_sends_its_clip_and_feather_in_set_and_animate_under_any_pose() {
        let mut h = crate::testing::harness();
        let fade = Edges::vertical(34.0);
        let mut part = plate_part(&h).clip_feathered(WINDOW, fade);
        h.commit();
        for pose in [Pose::REST, Pose::pressed(), Pose::slid(0.0, 16.0, 0.5)] {
            let p = part.props((140.0, 220.0), pose);
            assert_eq!((p.clip, p.feather), (Some(WINDOW), fade), "{pose:?}");
        }
        part.set((140.0, 220.0), Pose::slid(0.0, 16.0, 0.5));
        part.animate((140.0, 220.0), Pose::REST, 100, Ease::Out);
        let sent = h.commit();
        assert!(
            sent.iter().any(|r| matches!(
                r,
                Request::Set { props, .. } if props.clip == Some(WINDOW) && props.feather == fade
            )),
            "{sent:?}"
        );
        assert!(
            sent.iter().any(|r| matches!(
                r,
                Request::Animate { to, .. } if to.clip == Some(WINDOW) && to.feather == fade
            )),
            "{sent:?}"
        );
    }

    #[test]
    fn clipped_after_clip_feathered_is_a_hard_clip_again() {
        let h = crate::testing::harness();
        let fade = Edges::all(20.0);
        let feathered = plate_part(&h).clip_feathered(WINDOW, fade);
        assert_eq!(feathered.props((0.0, 0.0), Pose::REST).feather, fade);
        let hard = feathered.clipped(WINDOW);
        let p = hard.props((0.0, 0.0), Pose::REST);
        assert_eq!((p.clip, p.feather), (Some(WINDOW), Edges::NONE));
    }

    #[test]
    fn a_layer_enlarged_at_rest_stays_centred_and_a_pressed_group_shrinks_it_further() {
        // A 64 x 32 picture the compositor draws 15 times as large, centred on the screen.
        let picture = Frame::new(448.0, 224.0, 64.0, 32.0);
        let rest = Rest {
            scale: 15.0,
            ..Rest::PLAIN
        };
        let p = pose_props(picture, rest, (480.0, 240.0), Pose::REST);
        assert_eq!((p.x + 32.0, p.y + 16.0, p.scale), (480.0, 240.0, 15.0));
        let pressed = pose_props(picture, rest, (480.0, 240.0), Pose::pressed());
        assert!((pressed.scale - 15.0 * 0.97).abs() < 1e-4);
        assert_eq!((pressed.x + 32.0, pressed.y + 16.0), (480.0, 240.0));
    }
}
