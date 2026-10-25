//! Where things go on the panel: the rotation from the upright logical screen, layer quads, clip scissors and the
//! clamping of damage rectangles. Pure arithmetic, so the panel's orientation is testable without a GPU.

use compositor_protocol::{Edges, Rect};
use device_profile::Rotation;

/// A rectangle of whole pixels: layer pixels for damage, panel pixels for scissors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl PixRect {
    pub fn full(w: u32, h: u32) -> Self {
        Self { x: 0, y: 0, w, h }
    }

    /// The part both cover, if there is any.
    pub fn intersect(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        })
    }

    /// The pixels of a picture `factor` times the size (a texture a quarter as wide as the panel: 0.25) that cover this
    /// rectangle, rounded outwards so that no pixel of it is left out.
    pub fn scaled(self, factor: f32) -> Self {
        let left = (self.x as f32 * factor).floor() as u32;
        let top = (self.y as f32 * factor).floor() as u32;
        let right = ((self.x + self.w) as f32 * factor).ceil() as u32;
        let bottom = ((self.y + self.h) as f32 * factor).ceil() as u32;
        Self {
            x: left,
            y: top,
            w: (right - left).max(1),
            h: (bottom - top).max(1),
        }
    }

    /// The smallest rectangle holding both.
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.w).max(other.x + other.w);
        let bottom = (self.y + self.h).max(other.y + other.h);
        Self {
            x,
            y,
            w: right - x,
            h: bottom - y,
        }
    }
}

/// The pixels of a `width` × `height` layer that a damage rectangle touches: rounded outwards, so a partly
/// covered pixel counts, and clamped to the layer. `None` when nothing of it lies inside, or when it is not a
/// number.
pub fn clamp_damage(rect: Rect, width: u32, height: u32) -> Option<PixRect> {
    let values = [rect.x, rect.y, rect.w, rect.h];
    if values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (x, y, w, h) = (
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.w),
        f64::from(rect.h),
    );
    let left = x.floor().max(0.0);
    let top = y.floor().max(0.0);
    let right = (x + w).ceil().min(f64::from(width));
    let bottom = (y + h).ceil().min(f64::from(height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(PixRect {
        x: left as u32,
        y: top as u32,
        w: (right - left) as u32,
        h: (bottom - top) as u32,
    })
}

/// One corner of a layer quad: the panel position it is drawn at, and the layer-local position (logical pixels
/// from the layer's top-left) the shaders read the layer's content at.
pub type Vertex = [f32; 4];

/// A layer's rectangle on the logical screen after animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Uniform scale about the layer's centre.
    pub scale: f32,
}

/// A feathered clip on the panel: where the clip's two edges are along each panel axis, and how far in from each the fade
/// reaches. The fade at a panel position (x, y) is the product, over the two axes, of the least of the two edges' ramps
/// (0 at the edge, 1 at the fade's width in, linear between).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    /// `edge[axis]` is `[low, high]` along panel axis 0 (x) or 1 (y), in panel pixels.
    pub edge: [[f32; 2]; 2],
    /// `width[axis]` is how far in from `[low, high]` the fade reaches; 0 is a hard edge.
    pub width: [[f32; 2]; 2],
}

impl Ramp {
    /// The panel pixels whose centres the fade leaves whole, `[left, top, right, bottom]` (rounded inwards: a pixel in it
    /// is drawn exactly as it would be with no feather): nothing of the fade is in any pixel of a layer that lies inside it.
    pub fn solid(&self) -> [i32; 4] {
        // A pixel's centre is half a pixel on from its corner.
        let low = |axis: usize| (self.edge[axis][0] + self.width[axis][0] - 0.5).ceil() as i32;
        let high =
            |axis: usize| (self.edge[axis][1] - self.width[axis][1] - 0.5).floor() as i32 + 1;
        [low(0), low(1), high(0), high(1)]
    }
}

/// How the upright logical screen lands on the panel.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub rotation: Rotation,
    pub logical_width: f32,
    pub logical_height: f32,
    pub width: u32,
    pub height: u32,
}

impl Panel {
    /// The panel position of a continuous logical position. The continuous form of
    /// [`Rotation::panel_point`], which is the one definition of the rotation: pixel centres map to pixel centres.
    pub fn panel_position(&self, x: f32, y: f32) -> [f32; 2] {
        match self.rotation {
            Rotation::R0 => [x, y],
            Rotation::R90 => [y, self.logical_width - x],
            Rotation::R180 => [self.logical_width - x, self.logical_height - y],
            Rotation::R270 => [self.logical_height - y, x],
        }
    }

    /// The four corners of a whole layer as a triangle strip (top-left, top-right, bottom-left, bottom-right in layer
    /// space), grown by `pad` layer pixels on every side: the tests' shorthand for [`Panel::quad_of`].
    #[cfg(test)]
    pub fn quad(&self, at: &Placement, pad: f32) -> [Vertex; 4] {
        self.quad_of(at, [-pad, -pad, at.width + pad, at.height + pad])
    }

    /// The corners, as a triangle strip, of the part `rect` (`[left, top, right, bottom]` in layer pixels) of a layer:
    /// it may lie outside the layer, or be any piece of it.
    pub fn quad_of(&self, at: &Placement, rect: [f32; 4]) -> [Vertex; 4] {
        let (cx, cy) = (at.width / 2.0, at.height / 2.0);
        let corner = |lx: f32, ly: f32| -> Vertex {
            let sx = at.x + cx + (lx - cx) * at.scale;
            let sy = at.y + cy + (ly - cy) * at.scale;
            let [px, py] = self.panel_position(sx, sy);
            [px, py, lx, ly]
        };
        [
            corner(rect[0], rect[1]),
            corner(rect[2], rect[1]),
            corner(rect[0], rect[3]),
            corner(rect[2], rect[3]),
        ]
    }

    /// How many panel pixels one layer pixel is along the layer's x axis and along its y axis, for a layer drawn at
    /// `scale`, as `[xx, xy, yx, yy]`: where the panel moves when the layer position moves one pixel right, and
    /// when it moves one pixel down.
    pub fn axes(&self, scale: f32) -> [f32; 4] {
        let origin = self.panel_position(0.0, 0.0);
        let right = self.panel_position(1.0, 0.0);
        let down = self.panel_position(0.0, 1.0);
        [
            (right[0] - origin[0]) * scale,
            (right[1] - origin[1]) * scale,
            (down[0] - origin[0]) * scale,
            (down[1] - origin[1]) * scale,
        ]
    }

    /// The panel pixels a logical clip rectangle covers, rounded to the nearest pixel edges and limited to the
    /// panel. `None` when nothing of it is on the panel.
    pub fn scissor(&self, clip: Rect) -> Option<PixRect> {
        if [clip.x, clip.y, clip.w, clip.h]
            .iter()
            .any(|v| !v.is_finite())
        {
            return None;
        }
        let corners = [
            self.panel_position(clip.x, clip.y),
            self.panel_position(clip.x + clip.w, clip.y + clip.h),
        ];
        let edge = |axis: usize, limit: u32| {
            let lo = corners[0][axis].min(corners[1][axis]);
            let hi = corners[0][axis].max(corners[1][axis]);
            let limit = limit as f32;
            (lo.round().clamp(0.0, limit), hi.round().clamp(0.0, limit))
        };
        let (left, right) = edge(0, self.width);
        let (top, bottom) = edge(1, self.height);
        if right <= left || bottom <= top {
            return None;
        }
        Some(PixRect {
            x: left as u32,
            y: top as u32,
            w: (right - left) as u32,
            h: (bottom - top) as u32,
        })
    }

    /// The fade of a feathered logical clip as it lies on the panel, whichever way the panel is turned; `None` when there is
    /// nothing to fade (a hard clip, or a clip that is not a number).
    pub fn ramp(&self, clip: Rect, feather: Edges) -> Option<Ramp> {
        if feather.is_none()
            || !feather.is_valid()
            || [clip.x, clip.y, clip.w, clip.h]
                .iter()
                .any(|v| !v.is_finite())
        {
            return None;
        }
        let corners = [
            self.panel_position(clip.x, clip.y),
            self.panel_position(clip.x + clip.w, clip.y + clip.h),
        ];
        // Which panel axis the logical x axis (the left and right edges) and the logical y axis (top and bottom) run along,
        // and whether the panel's own direction on it is the logical one.
        let [xx, xy, yx, yy] = self.axes(1.0);
        let (along_x, x_forward) = if xx != 0.0 {
            (0, xx > 0.0)
        } else {
            (1, xy > 0.0)
        };
        let (along_y, y_forward) = if yx != 0.0 {
            (0, yx > 0.0)
        } else {
            (1, yy > 0.0)
        };
        let mut edge = [[0.0; 2]; 2];
        for (axis, edge) in edge.iter_mut().enumerate() {
            *edge = [
                corners[0][axis].min(corners[1][axis]),
                corners[0][axis].max(corners[1][axis]),
            ];
        }
        let mut width = [[0.0; 2]; 2];
        width[along_x] = if x_forward {
            [feather.left, feather.right]
        } else {
            [feather.right, feather.left]
        };
        width[along_y] = if y_forward {
            [feather.top, feather.bottom]
        } else {
            [feather.bottom, feather.top]
        };
        Some(Ramp { edge, width })
    }

    /// Whether any of the quad's bounding box lies inside `within` (the whole panel when `None`).
    pub fn is_on_panel(&self, quad: &[Vertex; 4], within: Option<PixRect>) -> bool {
        let (mut left, mut top) = (f32::MAX, f32::MAX);
        let (mut right, mut bottom) = (f32::MIN, f32::MIN);
        for v in quad {
            left = left.min(v[0]);
            right = right.max(v[0]);
            top = top.min(v[1]);
            bottom = bottom.max(v[1]);
        }
        let area = within.unwrap_or_else(|| PixRect::full(self.width, self.height));
        let (area_right, area_bottom) = ((area.x + area.w) as f32, (area.y + area.h) as f32);
        right > area.x as f32 && left < area_right && bottom > area.y as f32 && top < area_bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(rotation: Rotation, w: u32, h: u32) -> Panel {
        let (pw, ph) = if rotation.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        };
        Panel {
            rotation,
            logical_width: w as f32,
            logical_height: h as f32,
            width: pw,
            height: ph,
        }
    }

    const ROTATIONS: [Rotation; 4] = [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270];

    /// The panel pixel a logical pixel's centre is drawn into, by the compositor's own transform.
    fn drawn_pixel(p: &Panel, x: u32, y: u32) -> (u32, u32) {
        let [px, py] = p.panel_position(x as f32 + 0.5, y as f32 + 0.5);
        (px.floor() as u32, py.floor() as u32)
    }

    #[test]
    fn the_four_corners_of_the_echo_show_screen_land_where_the_profile_puts_them() {
        let p = panel(Rotation::R270, 960, 480);
        for (x, y) in [(0, 0), (959, 0), (0, 479), (959, 479)] {
            assert_eq!(
                drawn_pixel(&p, x, y),
                Rotation::R270.panel_point(x, y, 960, 480),
                "logical ({x}, {y})"
            );
        }
        // Top-left of the upright screen is the panel's top-right corner, and the screen's x runs down the panel.
        assert_eq!(drawn_pixel(&p, 0, 0), (479, 0));
        assert_eq!(drawn_pixel(&p, 959, 0), (479, 959));
    }

    #[test]
    fn every_pixel_lands_where_rotation_panel_point_says_for_every_rotation() {
        for rotation in ROTATIONS {
            let p = panel(rotation, 7, 4);
            for y in 0..4 {
                for x in 0..7 {
                    assert_eq!(
                        drawn_pixel(&p, x, y),
                        rotation.panel_point(x, y, 7, 4),
                        "{rotation:?} ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn a_full_screen_layer_covers_exactly_the_panel() {
        for rotation in ROTATIONS {
            let p = panel(rotation, 960, 480);
            let quad = p.quad(
                &Placement {
                    x: 0.0,
                    y: 0.0,
                    width: 960.0,
                    height: 480.0,
                    scale: 1.0,
                },
                0.0,
            );
            let (mut xs, mut ys): (Vec<f32>, Vec<f32>) = quad.iter().map(|v| (v[0], v[1])).unzip();
            xs.sort_by(f32::total_cmp);
            ys.sort_by(f32::total_cmp);
            assert_eq!((xs[0], xs[3]), (0.0, p.width as f32), "{rotation:?}");
            assert_eq!((ys[0], ys[3]), (0.0, p.height as f32), "{rotation:?}");
            // The layer's own coordinates ride along unrotated: the content is never mirrored.
            assert_eq!(quad[0][2..], [0.0, 0.0]);
            assert_eq!(quad[3][2..], [960.0, 480.0]);
        }
    }

    #[test]
    fn scale_grows_a_layer_about_its_centre_and_padding_grows_its_edge() {
        let p = panel(Rotation::R0, 960, 480);
        let at = Placement {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            scale: 2.0,
        };
        let quad = p.quad(&at, 0.0);
        assert_eq!(quad[0], [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(quad[3], [400.0, 200.0, 200.0, 100.0]);
        let padded = p.quad(&at, 1.0);
        assert_eq!(padded[0], [-2.0, -2.0, -1.0, -1.0]);
        assert_eq!(padded[3], [402.0, 202.0, 201.0, 101.0]);
    }

    #[test]
    fn a_quad_can_be_any_rectangle_of_the_layer_or_past_it() {
        let p = panel(Rotation::R0, 960, 480);
        let at = Placement {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            scale: 2.0,
        };
        // Left -1, top -2, right 203, bottom 104 layer pixels; the layer is drawn twice as large.
        let quad = p.quad_of(&at, [-1.0, -2.0, 203.0, 104.0]);
        assert_eq!(quad[0], [-2.0, -4.0, -1.0, -2.0]);
        assert_eq!(quad[3], [406.0, 208.0, 203.0, 104.0]);
        // A piece in the middle carries its own layer coordinates.
        let piece = p.quad_of(&at, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(piece[0], [20.0, 40.0, 10.0, 20.0]);
        assert_eq!(piece[3], [60.0, 80.0, 30.0, 40.0]);
    }

    #[test]
    fn the_axes_say_where_the_panel_moves_for_a_step_along_the_layer() {
        // R270: logical (x, y) is at panel (479 - y, x): a step right is a step down the panel, a step down the layer
        // is a step left.
        let p = panel(Rotation::R270, 960, 480);
        assert_eq!(p.axes(1.0), [0.0, 1.0, -1.0, 0.0]);
        assert_eq!(p.axes(2.5), [0.0, 2.5, -2.5, 0.0]);
        // They agree with the quad's own corners, whichever way the panel is turned.
        for rotation in ROTATIONS {
            let p = panel(rotation, 960, 480);
            let at = Placement {
                x: 30.0,
                y: 20.0,
                width: 100.0,
                height: 50.0,
                scale: 2.0,
            };
            let quad = p.quad(&at, 0.0);
            let axes = p.axes(2.0);
            // Top-left to top-right is the layer's width along x; top-left to bottom-left its height along y.
            let along_x = [quad[1][0] - quad[0][0], quad[1][1] - quad[0][1]];
            let along_y = [quad[2][0] - quad[0][0], quad[2][1] - quad[0][1]];
            assert_eq!(along_x, [axes[0] * 100.0, axes[1] * 100.0], "{rotation:?}");
            assert_eq!(along_y, [axes[2] * 50.0, axes[3] * 50.0], "{rotation:?}");
        }
    }

    #[test]
    fn a_feathered_clip_lands_on_the_panel_edge_by_edge_whichever_way_the_panel_is_turned() {
        // Four different widths: where each lands says which edge it was.
        let clip = rect(100.0, 60.0, 600.0, 300.0);
        let feather = Edges::new(1.0, 2.0, 3.0, 4.0);
        for rotation in ROTATIONS {
            let p = panel(rotation, 960, 480);
            let ramp = p.ramp(clip, feather).expect("a feather");
            // The middle of each edge of the clip, and a step inside it from there, on the panel.
            for (name, edge, inside, width) in [
                ("left", (100.0, 210.0), (101.0, 210.0), 1.0),
                ("right", (700.0, 210.0), (699.0, 210.0), 2.0),
                ("top", (400.0, 60.0), (400.0, 61.0), 3.0),
                ("bottom", (400.0, 360.0), (400.0, 359.0), 4.0),
            ] {
                let on = p.panel_position(edge.0, edge.1);
                let step = p.panel_position(inside.0, inside.1);
                // The axis the edge is a line of is the one the step inside moves along, and the edge is its low one when
                // that step goes up.
                let axis = usize::from((step[0] - on[0]).abs() < 0.5);
                let side = usize::from(step[axis] < on[axis]);
                assert_eq!(ramp.edge[axis][side], on[axis], "{rotation:?} {name}");
                assert_eq!(ramp.width[axis][side], width, "{rotation:?} {name}");
            }
            assert_eq!(
                ramp.width.iter().flatten().sum::<f32>(),
                10.0,
                "{rotation:?}"
            );
        }
        let p = panel(Rotation::R270, 960, 480);
        assert_eq!(p.ramp(clip, Edges::NONE), None);
        assert_eq!(p.ramp(clip, Edges::new(-1.0, 0.0, 0.0, 0.0)), None);
        assert_eq!(p.ramp(rect(f32::NAN, 0.0, 1.0, 1.0), feather), None);
    }

    #[test]
    fn a_fade_leaves_whole_the_pixels_whose_centres_are_a_width_in() {
        let p = panel(Rotation::R0, 960, 480);
        let ramp = p
            .ramp(
                rect(100.0, 60.0, 600.0, 300.0),
                Edges::new(40.0, 24.0, 30.0, 16.0),
            )
            .unwrap();
        // The first pixel whose centre is 40 in from the edge at 100 is 140 (centre 140.5; 139 is at 39.5), and the last
        // before 24 from 700 is 675 (centre 675.5): the area ends at 676.
        assert_eq!(ramp.solid(), [140, 90, 676, 344]);
        // An edge with no fade is whole up to the edge itself.
        let hard = p.ramp(
            rect(100.0, 60.0, 600.0, 300.0),
            Edges::new(0.0, 24.0, 0.0, 0.0),
        );
        assert_eq!(hard.unwrap().solid(), [100, 60, 676, 360]);
        // And on the Show's turned panel the same clip is the same pixels, turned.
        let show = panel(Rotation::R270, 960, 480);
        let ramp = show
            .ramp(
                rect(100.0, 60.0, 600.0, 300.0),
                Edges::new(40.0, 24.0, 30.0, 16.0),
            )
            .unwrap();
        // Panel x is 480 - y: the top fade (30) is the high side of x, the bottom (16) its low side; panel y is x.
        assert_eq!(ramp.solid(), [120 + 16, 140, 420 - 30, 676]);
    }

    #[test]
    fn a_logical_clip_becomes_the_rotated_panel_rectangle() {
        let p = panel(Rotation::R270, 960, 480);
        // The slider fill of the hardware test: x from 100 to 400, y from 200 to 240 on the upright screen.
        let s = p
            .scissor(Rect {
                x: 100.0,
                y: 200.0,
                w: 300.0,
                h: 40.0,
            })
            .unwrap();
        assert_eq!(
            s,
            PixRect {
                x: 240,
                y: 100,
                w: 40,
                h: 300
            }
        );
        // Every logical pixel inside the clip is inside the scissor after the rotation, and its neighbours are not.
        let inside = Rotation::R270.panel_point(100, 200, 960, 480);
        let outside = Rotation::R270.panel_point(99, 200, 960, 480);
        let contains = |(x, y): (u32, u32)| x >= s.x && x < s.x + s.w && y >= s.y && y < s.y + s.h;
        assert!(contains(inside));
        assert!(!contains(outside));
    }

    #[test]
    fn a_clip_is_limited_to_the_panel_and_an_empty_one_shows_nothing() {
        let p = panel(Rotation::R0, 960, 480);
        let s = p
            .scissor(Rect {
                x: -50.0,
                y: 400.0,
                w: 2000.0,
                h: 500.0,
            })
            .unwrap();
        assert_eq!(
            s,
            PixRect {
                x: 0,
                y: 400,
                w: 960,
                h: 80
            }
        );
        for clip in [
            Rect {
                x: 10.0,
                y: 10.0,
                w: 0.0,
                h: 50.0,
            },
            Rect {
                x: 2000.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                w: -5.0,
                h: 10.0,
            },
            Rect {
                x: f32::NAN,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        ] {
            assert_eq!(p.scissor(clip), None, "{clip:?}");
        }
    }

    #[test]
    fn layers_off_the_panel_or_outside_their_clip_are_recognised() {
        let p = panel(Rotation::R0, 960, 480);
        let at = |x: f32| Placement {
            x,
            y: 10.0,
            width: 100.0,
            height: 50.0,
            scale: 1.0,
        };
        assert!(p.is_on_panel(&p.quad(&at(0.0), 0.0), None));
        assert!(p.is_on_panel(&p.quad(&at(-99.0), 0.0), None));
        assert!(!p.is_on_panel(&p.quad(&at(-100.0), 0.0), None));
        assert!(!p.is_on_panel(&p.quad(&at(960.0), 0.0), None));
        let clip = Some(PixRect {
            x: 500,
            y: 0,
            w: 100,
            h: 100,
        });
        assert!(!p.is_on_panel(&p.quad(&at(0.0), 0.0), clip));
        assert!(p.is_on_panel(&p.quad(&at(450.0), 0.0), clip));
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn damage_is_rounded_outwards_and_kept_inside_the_layer() {
        assert_eq!(
            clamp_damage(rect(1.2, 2.7, 3.0, 1.0), 100, 50),
            Some(PixRect {
                x: 1,
                y: 2,
                w: 4,
                h: 2
            })
        );
        assert_eq!(
            clamp_damage(rect(-10.0, -10.0, 30.0, 30.0), 100, 50),
            Some(PixRect {
                x: 0,
                y: 0,
                w: 20,
                h: 20
            })
        );
        assert_eq!(
            clamp_damage(rect(90.0, 40.0, 1e30, 1e30), 100, 50),
            Some(PixRect {
                x: 90,
                y: 40,
                w: 10,
                h: 10
            })
        );
    }

    #[test]
    fn damage_with_nothing_inside_the_layer_is_dropped() {
        for r in [
            rect(100.0, 0.0, 10.0, 10.0),
            rect(0.0, 50.0, 10.0, 10.0),
            rect(-20.0, 0.0, 10.0, 10.0),
            rect(5.0, 5.0, 0.0, 10.0),
            rect(5.0, 5.0, -3.0, 10.0),
            rect(f32::NAN, 0.0, 10.0, 10.0),
            rect(0.0, 0.0, f32::INFINITY, 10.0),
        ] {
            assert_eq!(clamp_damage(r, 100, 50), None, "{r:?}");
        }
    }

    #[test]
    fn a_union_of_damage_is_the_bounding_box() {
        let a = PixRect {
            x: 10,
            y: 10,
            w: 5,
            h: 5,
        };
        let b = PixRect {
            x: 40,
            y: 2,
            w: 10,
            h: 4,
        };
        assert_eq!(
            a.union(b),
            PixRect {
                x: 10,
                y: 2,
                w: 40,
                h: 13
            }
        );
        assert_eq!(a.union(a), a);
    }

    #[test]
    fn two_rectangles_share_the_part_they_both_cover_and_nothing_when_they_only_touch() {
        let a = PixRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
        };
        let b = PixRect {
            x: 20,
            y: 0,
            w: 30,
            h: 15,
        };
        assert_eq!(
            a.intersect(b),
            Some(PixRect {
                x: 20,
                y: 10,
                w: 10,
                h: 5
            })
        );
        assert_eq!(a.intersect(a), Some(a));
        // Side by side, sharing only an edge, or far apart.
        assert_eq!(
            a.intersect(PixRect {
                x: 30,
                y: 10,
                w: 5,
                h: 5
            }),
            None
        );
        assert_eq!(
            a.intersect(PixRect {
                x: 100,
                y: 100,
                w: 5,
                h: 5
            }),
            None
        );
    }

    #[test]
    fn a_rectangle_scaled_to_a_smaller_picture_is_rounded_outwards_and_never_empty() {
        let r = PixRect {
            x: 5,
            y: 9,
            w: 10,
            h: 10,
        };
        // A quarter: x 1.25..3.75 covers texels 1..4, y 2.25..4.75 covers 2..5.
        assert_eq!(
            r.scaled(0.25),
            PixRect {
                x: 1,
                y: 2,
                w: 3,
                h: 3
            }
        );
        assert_eq!(r.scaled(1.0), r);
        // A one-pixel sliver still covers a texel.
        assert_eq!(
            PixRect {
                x: 7,
                y: 7,
                w: 1,
                h: 1
            }
            .scaled(0.125),
            PixRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1
            }
        );
    }
}
