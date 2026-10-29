//! A host reference of what the compositor makes of the requests an app sent: where every layer is and how opaque, at a moment after
//! the settles and animations that were asked for began, and the picture the image layers of it make. Test support: it reads the
//! requests a recording compositor (`testing::Harness`, or the one of a loop test) was told, and nothing else of the app.
//!
//! It is the compositor's placement and no more: the tree of groups and the pose of each, evaluated with the same code the compositor
//! and the toolkit's own estimates run (`compositor_protocol::group` and `motion`: tracks, bindings, worlds), a layer's own animation
//! (position, scale, opacity along its curve), stacking by `z` and creation order, and the layers that are drawn at all (shown, not
//! hidden, not transparent). Pixels are only the image layers', blended premultiplied over an opaque canvas ([`composite`]); a
//! shape, a piece of glass or a run of glyphs that is shown is named, not drawn.
//!
//! Every request counts as made at `begun_ns` (the moment a settle or an animation starts) and everything is read at `at_ns`, so it
//! serves one commit's worth of motion: the first frame of a screen and the frames after it.

use compositor_protocol::group::World;
use compositor_protocol::motion::{Track, ease};
use compositor_protocol::{
    Content, Ease, GroupId, GroupPose, LayerId, MotionId, PoseBinding, PoseProperties, Props,
    Request,
};
use std::collections::HashMap;

/// A layer as the compositor draws it at the moment asked about: in screen space, with the poses of the groups it hangs under in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Flat {
    pub id: LayerId,
    pub z: i32,
    pub width: u32,
    pub height: u32,
    pub content: Content,
    pub props: Props,
}

impl Flat {
    /// Whether the compositor draws it at all: not hidden, not transparent, not scaled to nothing.
    pub fn shown(&self) -> bool {
        self.props.visible && self.props.opacity > 0.0 && self.props.scale > 0.0
    }
}

/// A layer's own animation, from the props it had when it was asked for.
struct Going {
    from: Props,
    to: Props,
    duration_ms: u32,
    curve: Ease,
}

struct Layer {
    id: LayerId,
    width: u32,
    height: u32,
    z: i32,
    content: Content,
    /// In the group's space under a group.
    props: Props,
    going: Option<Going>,
    group: Option<GroupId>,
}

const PROPERTIES: [PoseProperties; 3] = [
    PoseProperties::TRANSLATION,
    PoseProperties::SCALE,
    PoseProperties::OPACITY,
];

struct Group {
    parent: Option<GroupId>,
    pose: GroupPose,
    bound: [Option<(MotionId, PoseBinding)>; 3],
}

/// The layers `requests` leave on the screen at `at_ns`, back to front (by `z`, and by creation among layers of one `z`).
pub fn flatten(requests: &[Request], begun_ns: u64, at_ns: u64) -> Vec<Flat> {
    let mut layers: Vec<Layer> = Vec::new();
    let mut groups: HashMap<GroupId, Group> = HashMap::new();
    let mut tracks: HashMap<MotionId, Track> = HashMap::new();
    let elapsed = at_ns.saturating_sub(begun_ns);
    let at = |layers: &mut Vec<Layer>, id: LayerId| layers.iter().position(|l| l.id == id);
    for request in requests {
        match request {
            Request::Create {
                id,
                width,
                height,
                z,
                content,
                props,
            } => {
                layers.retain(|l| l.id != *id);
                layers.push(Layer {
                    id: *id,
                    width: *width,
                    height: *height,
                    z: *z,
                    content: *content,
                    props: *props,
                    going: None,
                    group: None,
                });
            }
            Request::Set { id, props } => {
                if let Some(i) = at(&mut layers, *id) {
                    layers[i].props = *props;
                    layers[i].going = None;
                }
            }
            Request::Animate {
                id,
                to,
                duration_ms,
                ease,
            } => {
                if let Some(i) = at(&mut layers, *id) {
                    // From where the layer is when the animation is asked for: a layer asked twice goes on from where it was.
                    let from = layer_props(&layers[i], elapsed);
                    layers[i].props = from;
                    layers[i].going = Some(Going {
                        from,
                        to: *to,
                        duration_ms: *duration_ms,
                        curve: *ease,
                    });
                }
            }
            Request::Restyle { id, content } => {
                if let Some(i) = at(&mut layers, *id) {
                    layers[i].content = *content;
                }
            }
            Request::Delete { id } => layers.retain(|l| l.id != *id),
            Request::Attach { id, group, local } => {
                if let Some(i) = at(&mut layers, *id) {
                    layers[i].group = *group;
                    layers[i].props = *local;
                    layers[i].going = None;
                }
            }
            Request::GroupCreate { id, parent, pose } => {
                groups.insert(
                    *id,
                    Group {
                        parent: *parent,
                        pose: *pose,
                        bound: [None; 3],
                    },
                );
            }
            Request::GroupSet { id, pose } => {
                if let Some(g) = groups.get_mut(id) {
                    g.pose = *pose;
                }
            }
            Request::GroupParent { id, parent, .. } => {
                if let Some(g) = groups.get_mut(id) {
                    g.parent = *parent;
                }
            }
            Request::GroupBind {
                id,
                motion,
                binding,
            } => {
                if let Some(g) = groups.get_mut(id) {
                    for (slot, property) in g.bound.iter_mut().zip(PROPERTIES) {
                        if binding.properties.contains(property) {
                            *slot = Some((
                                *motion,
                                PoseBinding {
                                    properties: property,
                                    ..*binding
                                },
                            ));
                        }
                    }
                }
            }
            Request::GroupUnbind { id, properties } => {
                // The properties keep the values they have: the track's, as of the moment asked about.
                if let Some(mut g) = groups.remove(id) {
                    for (slot, property) in g.bound.iter_mut().zip(PROPERTIES) {
                        if let (true, Some((motion, binding))) =
                            (properties.contains(property), *slot)
                        {
                            if let Some(track) = tracks.get(&motion) {
                                let mut track = *track;
                                track.advance(at_ns);
                                binding.apply(track.value(), &mut g.pose);
                            }
                            *slot = None;
                        }
                    }
                    groups.insert(*id, g);
                }
            }
            Request::GroupDelete { id } => {
                groups.remove(id);
            }
            Request::MotionCreate { id, value, bounds } => {
                tracks.insert(*id, Track::new(*value, *bounds));
            }
            Request::MotionSettle {
                id,
                target,
                velocity,
                settle,
            } => {
                if let Some(track) = tracks.get_mut(id) {
                    track.settle(begun_ns, *target, *velocity, *settle);
                }
            }
            Request::MotionCancel { id, policy } => {
                if let Some(track) = tracks.get_mut(id) {
                    track.cancel(begun_ns, *policy);
                }
            }
            Request::MotionDelete { id } => {
                tracks.remove(id);
            }
            _ => {}
        }
    }
    for track in tracks.values_mut() {
        track.advance(at_ns);
    }
    let mut worlds: HashMap<GroupId, World> = HashMap::new();
    let mut out: Vec<Flat> = layers
        .iter()
        .map(|layer| {
            let local = layer_props(layer, elapsed);
            let props = match layer.group {
                Some(g) => world_of(g, &groups, &tracks, &mut worlds)
                    .props(&local, (layer.width as f32, layer.height as f32)),
                None => local,
            };
            Flat {
                id: layer.id,
                z: layer.z,
                width: layer.width,
                height: layer.height,
                content: layer.content,
                props,
            }
        })
        .collect();
    out.sort_by_key(|l| l.z);
    out
}

/// The layer's own props `elapsed_ns` after its animation began.
fn layer_props(layer: &Layer, elapsed_ns: u64) -> Props {
    let Some(going) = &layer.going else {
        return layer.props;
    };
    let ms = elapsed_ns as f64 / 1e6;
    if going.duration_ms == 0 || ms >= f64::from(going.duration_ms) {
        return going.to;
    }
    let t = ease(going.curve, (ms / f64::from(going.duration_ms)) as f32);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Props {
        x: lerp(going.from.x, going.to.x),
        y: lerp(going.from.y, going.to.y),
        scale: lerp(going.from.scale, going.to.scale),
        opacity: lerp(going.from.opacity, going.to.opacity),
        clip: going.to.clip,
        feather: going.to.feather,
        visible: going.from.visible || going.to.visible,
    }
}

/// The world a group is drawn in: its ancestors' poses and its own, composed from the screen down, each pose with the properties bound
/// to a track at that track's value.
fn world_of(
    id: GroupId,
    groups: &HashMap<GroupId, Group>,
    tracks: &HashMap<MotionId, Track>,
    memo: &mut HashMap<GroupId, World>,
) -> World {
    if let Some(world) = memo.get(&id) {
        return *world;
    }
    let Some(group) = groups.get(&id) else {
        return World::ROOT;
    };
    let parent = group
        .parent
        .map_or(World::ROOT, |p| world_of(p, groups, tracks, memo));
    let mut pose = group.pose;
    for (motion, binding) in group.bound.iter().flatten() {
        if let Some(track) = tracks.get(motion) {
            binding.apply(track.value(), &mut pose);
        }
    }
    let world = parent.child(&pose);
    memo.insert(id, world);
    world
}

/// The picture of the image layers among `layers` (back to front, as [`flatten`] gives them) over an opaque canvas of `size`, three bytes
/// a pixel, row after row, with `pixels` giving a layer's premultiplied RGBA (`width * 4` bytes a row). A layer that is shown and not an
/// image, and an image that is clipped, cannot be drawn here: the answer is then `Err` with the layers that cannot. An image that is
/// scaled (about its centre) is sampled linearly, texel centres at half integers and the edge texels held, as the compositor does.
/// Layers below an image that is opaque, whole-screen, unscaled and fully shown are covered by it and left out, as the compositor
/// leaves them out.
pub fn composite(
    layers: &[Flat],
    size: (u32, u32),
    pixels: impl Fn(LayerId) -> Vec<u8>,
) -> Result<Vec<u8>, Vec<LayerId>> {
    let (w, h) = (size.0 as usize, size.1 as usize);
    let shown: Vec<&Flat> = layers.iter().filter(|l| l.shown()).collect();
    let covers = |l: &Flat, picture: &[u8]| {
        l.content == Content::Image
            && l.props.opacity >= 1.0
            && l.props.scale == 1.0
            && l.props.clip.is_none()
            && (l.width as usize, l.height as usize) == (w, h)
            && (l.props.x, l.props.y) == (0.0, 0.0)
            && picture.chunks_exact(4).all(|px| px[3] == 255)
    };
    let mut start = 0;
    for (i, l) in shown.iter().enumerate() {
        if l.content == Content::Image && covers(l, &pixels(l.id)) {
            start = i;
        }
    }
    let mut cannot = Vec::new();
    for l in &shown[start..] {
        let plain = l.content == Content::Image && l.props.scale > 0.0 && l.props.clip.is_none();
        if !plain {
            cannot.push(l.id);
        }
    }
    if !cannot.is_empty() {
        return Err(cannot);
    }
    let mut canvas = vec![0.0f32; w * h * 3];
    for l in &shown[start..] {
        let picture = pixels(l.id);
        if l.props.scale != 1.0 {
            draw_scaled(&mut canvas, (w, h), l, &picture);
            continue;
        }
        let (x0, y0) = (l.props.x.round() as i64, l.props.y.round() as i64);
        for ly in 0..l.height as i64 {
            for lx in 0..l.width as i64 {
                let (x, y) = (x0 + lx, y0 + ly);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let at = (ly as usize * l.width as usize + lx as usize) * 4;
                let a = f32::from(picture[at + 3]) / 255.0 * l.props.opacity;
                let to = (y as usize * w + x as usize) * 3;
                for c in 0..3 {
                    let src = f32::from(picture[at + c]) * l.props.opacity;
                    canvas[to + c] = src + canvas[to + c] * (1.0 - a);
                }
            }
        }
    }
    Ok(canvas
        .into_iter()
        .map(|v| (v + 0.5).clamp(0.0, 255.0) as u8)
        .collect())
}

/// `l`, an image scaled about its centre, over `canvas` (`size`, three floats a pixel): each pixel it covers takes the layer's picture
/// at the middle of that pixel, sampled linearly between the four nearest texel centres.
fn draw_scaled(canvas: &mut [f32], size: (usize, usize), l: &Flat, picture: &[u8]) {
    let (w, h) = size;
    let (lw, lh) = (l.width as usize, l.height as usize);
    let s = l.props.scale;
    let (cx, cy) = (
        l.props.x + l.width as f32 / 2.0,
        l.props.y + l.height as f32 / 2.0,
    );
    let (half_w, half_h) = (l.width as f32 * s / 2.0, l.height as f32 * s / 2.0);
    let x_range =
        (cx - half_w).floor().max(0.0) as usize..((cx + half_w).ceil().max(0.0) as usize).min(w);
    let y_range =
        (cy - half_h).floor().max(0.0) as usize..((cy + half_h).ceil().max(0.0) as usize).min(h);
    for y in y_range {
        for x in x_range.clone() {
            let u = ((x as f32 + 0.5 - cx) / s + l.width as f32 / 2.0 - 0.5)
                .clamp(0.0, (lw - 1) as f32);
            let v = ((y as f32 + 0.5 - cy) / s + l.height as f32 / 2.0 - 0.5)
                .clamp(0.0, (lh - 1) as f32);
            let (u0, v0) = (u.floor() as usize, v.floor() as usize);
            let (u1, v1) = ((u0 + 1).min(lw - 1), (v0 + 1).min(lh - 1));
            let (fu, fv) = (u - u0 as f32, v - v0 as f32);
            let texel = |a: usize, b: usize, c: usize| f32::from(picture[(b * lw + a) * 4 + c]);
            let mut sample = [0f32; 4];
            for (c, out) in sample.iter_mut().enumerate() {
                let top = texel(u0, v0, c) * (1.0 - fu) + texel(u1, v0, c) * fu;
                let bottom = texel(u0, v1, c) * (1.0 - fu) + texel(u1, v1, c) * fu;
                *out = top * (1.0 - fv) + bottom * fv;
            }
            let a = sample[3] / 255.0 * l.props.opacity;
            let to = (y * w + x) * 3;
            for c in 0..3 {
                canvas[to + c] = sample[c] * l.props.opacity + canvas[to + c] * (1.0 - a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::{Fill, MotionBounds, MotionToken, Rgba, Settle, Shape, Units};

    fn shape() -> Content {
        Content::Shape(Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(1, 2, 3, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        })
    }

    fn make(id: u32, z: i32, props: Props) -> Request {
        Request::Create {
            id: LayerId(id),
            width: 10,
            height: 10,
            z,
            content: shape(),
            props,
        }
    }

    const MS: u64 = 1_000_000;

    /// A layer under a group whose pose follows a track that is settling: the reference moves it as the compositor's groups do.
    #[test]
    fn a_group_bound_to_a_settling_track_carries_its_layers_along_it() {
        let rise = PoseBinding {
            values: [0.0, 1.0],
            from: GroupPose {
                opacity: 0.0,
                ..GroupPose::at(0.0, 24.0)
            },
            to: GroupPose::IDENTITY,
            properties: PoseProperties::TRANSLATION.union(PoseProperties::OPACITY),
        };
        let requests = [
            Request::GroupCreate {
                id: GroupId(1),
                parent: None,
                pose: GroupPose::IDENTITY,
            },
            Request::MotionCreate {
                id: MotionId(1),
                value: 0.0,
                bounds: MotionBounds::range(0.0, 1.0, Units::Progress),
            },
            Request::GroupBind {
                id: GroupId(1),
                motion: MotionId(1),
                binding: rise,
            },
            make(1, 0, Props::at(5.0, 7.0)),
            Request::Attach {
                id: LayerId(1),
                group: Some(GroupId(1)),
                local: Props::at(5.0, 7.0),
            },
            Request::MotionSettle {
                id: MotionId(1),
                target: Some(1.0),
                velocity: None,
                settle: Settle::Timed(MotionToken {
                    duration_ms: 100,
                    ease: Ease::Linear,
                }),
            },
        ];
        let at = |ms: u64| flatten(&requests, 0, ms * MS)[0].props;
        assert_eq!((at(0).y, at(0).opacity), (31.0, 0.0));
        assert!((at(50).y - 19.0).abs() < 1e-3 && (at(50).opacity - 0.5).abs() < 1e-3);
        assert_eq!((at(100).x, at(100).y, at(100).opacity), (5.0, 7.0, 1.0));
        assert!(!flatten(&requests, 0, 0)[0].shown());
    }

    #[test]
    fn a_layers_own_animation_runs_from_where_it_was_and_stacking_is_by_z_then_creation() {
        let requests = [
            make(1, 0, Props::at(0.0, 0.0)),
            make(2, -1, Props::at(0.0, 0.0)),
            Request::Animate {
                id: LayerId(1),
                to: Props::at(100.0, 0.0),
                duration_ms: 200,
                ease: Ease::Linear,
            },
        ];
        let flat = flatten(&requests, 0, 100 * MS);
        assert_eq!(flat.iter().map(|l| l.id.0).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(flat[1].props.x, 50.0);
    }

    /// An image scaled about its centre is drawn as the compositor draws it: linear sampling, texel centres at half integers, the edge
    /// texels held beyond them.
    #[test]
    fn an_image_scaled_about_its_centre_is_sampled_linearly_with_its_edge_texels_held() {
        // Two texels, red and blue, at x = 2 and four times their size: centre 3, spanning -1 to 7 over a canvas six pixels wide.
        let flat = Flat {
            id: LayerId(1),
            z: 0,
            width: 2,
            height: 1,
            content: Content::Image,
            props: Props {
                scale: 4.0,
                ..Props::at(2.0, 0.0)
            },
        };
        let picture = vec![255, 0, 0, 255, 0, 0, 255, 255];
        let drawn = composite(&[flat], (6, 1), |_| picture.clone()).unwrap();
        let px = |x: usize| [drawn[x * 3], drawn[x * 3 + 1], drawn[x * 3 + 2]];
        assert_eq!(
            px(0),
            [255, 0, 0],
            "held red beyond the first texel's centre"
        );
        assert_eq!(px(5), [0, 0, 255], "held blue beyond the second's");
        // Pixel 3 is at 0.625 of the way from the red texel's centre to the blue's: 37.5 % red.
        assert_eq!(px(3), [96, 0, 159]);
        assert_eq!(px(2), [159, 0, 96], "and pixel 2 at 0.375");
    }
}
