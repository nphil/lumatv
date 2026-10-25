//! Prepared backdrops: sets of blurred plates made ahead of the glass that shows them (design §3.1 item 3 and §3.2;
//! `compositor_protocol::resources` is the contract).
//!
//! A *set* is a wallpaper plate and one plate for each group the app names. A *generation* is a finished set: every plate
//! drawn from the scene as it was when its turn came and blurred with the same chain a glass surface's own backdrop uses
//! (`backdrop::Backdrops::blurred`), published all at once. Generations are made in steps, a plate at a time, in moments the
//! compositor has to spare (`Renderer::prepare`); the one being made and the one shown are the two a set ever has.
//!
//! A *plate* is a texture of the group's own space: the layers of the group as they are drawn with the group's pose taken as
//! the identity, at `1/downsample` of the panel, with a margin all round that the blur spreads into (so that a plate that is
//! moved has the soft edge of the blur and not a cut). Nothing of the group is drawn in the margin: the plate holds what
//! the group shows on the screen. It is shown where its group is *now*: the plates are drawn into the set's *picture*, a
//! small texture covering the panel that the glass surfaces read like any backdrop, each placed by the world of its group
//! (transform, opacity, visibility and clip) over the wallpaper plate. The picture is composed again only when a world or
//! the generation changes, and a still screen costs nothing.
//!
//! This file holds the sets and the arithmetic of their plates; the drawing is the renderer's (`renderer/prepare.rs`).

use crate::backdrop::{Backdrops, BlurPlan};
use crate::geometry::{Panel, Vertex};
use crate::gl::{Gles, RenderTarget};
use crate::log::log;
use compositor_protocol::group::World;
use compositor_protocol::{BackdropId, CaptureSpec, Event, GroupId, LayerId};
use device_profile::Rotation;
use std::collections::BTreeMap;

/// How big a set's plates are, worked out when the set is made from the panel and the spec.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The blur chain's settings.
    pub plan: BlurPlan,
    pub downsample: u8,
    /// Panel pixels to a texel of a finished plate and of the picture: `downsample` doubled once for every level the
    /// blur chain is left short of the top (`BlurPlan::result`).
    pub texel: u32,
    /// The margin round a group's plate, in panel pixels: whole texels of the finished plate, so that they line up with the
    /// picture's, and as far as the blur reaches.
    pub margin: u32,
    /// What the first level of a group plate's chain is a picture of, in panel pixels: the panel and its margins.
    pub group_size: (u32, u32),
    /// The size of a finished group plate, and of the wallpaper's plate (which has no margin: the blur holds its edge, as
    /// that of a glass surface's own backdrop does), in texels. The wallpaper's plate is also the size of the picture.
    pub group_plate: (u32, u32),
    pub wallpaper_plate: (u32, u32),
}

impl Geometry {
    pub fn of(backdrops: &Backdrops, panel: (u32, u32), radius: f32, downsample: u8) -> Self {
        let downsample = downsample.max(1);
        let plan = backdrops.plan_for(radius, downsample);
        let texel = u32::from(downsample) << plan.result;
        let reach = plan.reach(downsample).max(0) as u32;
        let margin = reach.div_ceil(texel) * texel;
        let group_size = (panel.0 + 2 * margin, panel.1 + 2 * margin);
        Self {
            plan,
            downsample,
            texel,
            margin,
            group_size,
            group_plate: Backdrops::finished_size(plan, group_size, downsample),
            wallpaper_plate: Backdrops::finished_size(plan, panel, downsample),
        }
    }
}

/// A finished set: its plates, wallpaper first when the set has one, then the groups in the spec's order. `None` where
/// there was nothing to draw.
struct Generation {
    number: u64,
    plates: Vec<Option<RenderTarget>>,
}

/// A generation being made.
struct Build {
    content: u64,
    /// The plates finished so far, in the same order.
    plates: Vec<Option<RenderTarget>>,
}

/// What the picture holds: which generation, and the worlds its plates were placed by.
struct Composed {
    generation: u64,
    worlds: Vec<Option<World>>,
}

struct Set {
    spec: CaptureSpec,
    geometry: Geometry,
    /// The number the next generation gets (the first is 1).
    next: u64,
    ready: Option<Generation>,
    /// Freezes held: no generation is made, and the shown one is not replaced, while there is any.
    leases: u32,
    /// The newest content asked for and not yet made.
    dirty: Option<u64>,
    build: Option<Build>,
    /// The picture the glass reads, made when the first generation is published.
    picture: Option<RenderTarget>,
    composed: Option<Composed>,
    /// The worlds of the spec's groups as the frame last saw them.
    worlds: Vec<Option<World>>,
    /// Changes whenever what the picture shows does (a generation is published, a world changes).
    revision: u64,
    /// The frame that last looked at the worlds, and the last that had glass showing the set.
    observed: u64,
    used: u64,
}

impl Set {
    fn wallpaper_offset(&self) -> usize {
        usize::from(self.spec.wallpaper.is_some())
    }

    /// How many plates a generation has.
    fn plate_count(&self) -> usize {
        self.wallpaper_offset() + self.spec.plates.len()
    }

    fn give_back(self, backdrops: &mut Backdrops) {
        let held = self
            .ready
            .into_iter()
            .flat_map(|g| g.plates)
            .chain(self.build.into_iter().flat_map(|b| b.plates))
            .flatten()
            .chain(self.picture);
        for target in held {
            backdrops.give(target);
        }
    }
}

/// What a plate is a plate of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plate {
    /// The wallpaper layer.
    Wallpaper(LayerId),
    /// The layers of a group.
    Group(GroupId),
}

/// One step of making a generation.
#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub id: BackdropId,
    /// The plate to draw; `None` for a set that has nothing in it, whose first step only publishes.
    pub plate: Option<Plate>,
    pub geometry: Geometry,
    pub radius: f32,
    /// The plate is the last: the generation is published after it.
    pub last: bool,
}

/// A generation published.
pub struct Published {
    pub event: Event,
    /// The number of the last frame that had glass showing the set (0: none yet, or the set has no picture to show): that
    /// glass is out of date until a frame is drawn.
    pub glass_frame: u64,
}

#[derive(Default)]
pub struct Sets {
    sets: BTreeMap<BackdropId, Set>,
}

impl Sets {
    /// Makes the set; one of that name that exists is replaced and its textures given back.
    pub fn create(
        &mut self,
        id: BackdropId,
        spec: CaptureSpec,
        geometry: Geometry,
        backdrops: &mut Backdrops,
    ) {
        self.delete(id, backdrops);
        let worlds = vec![None; spec.plates.len()];
        self.sets.insert(
            id,
            Set {
                spec,
                geometry,
                next: 1,
                ready: None,
                leases: 0,
                dirty: None,
                build: None,
                picture: None,
                composed: None,
                worlds,
                revision: 0,
                observed: 0,
                used: 0,
            },
        );
    }

    /// Frees everything the set holds. A set that is not there is nobody's.
    pub fn delete(&mut self, id: BackdropId, backdrops: &mut Backdrops) {
        if let Some(set) = self.sets.remove(&id) {
            set.give_back(backdrops);
        }
    }

    pub fn clear(&mut self, backdrops: &mut Backdrops) {
        for (_, set) in std::mem::take(&mut self.sets) {
            set.give_back(backdrops);
        }
    }

    /// `BackdropRefresh`: the newest content asked for is the one that will be made.
    pub fn refresh(&mut self, id: BackdropId, content: u64) {
        if let Some(set) = self.sets.get_mut(&id) {
            set.dirty = Some(content);
        }
    }

    /// `BackdropFreeze`: takes a lease on the generation shown (the newest finished one) and answers with its number (0:
    /// none yet). A generation being made is thrown away, and made again when the last lease is let go.
    pub fn freeze(&mut self, id: BackdropId, events: &mut Vec<Event>, backdrops: &mut Backdrops) {
        let Some(set) = self.sets.get_mut(&id) else {
            return;
        };
        set.leases += 1;
        if let Some(build) = set.build.take() {
            set.dirty = Some(
                set.dirty
                    .map_or(build.content, |newer| newer.max(build.content)),
            );
            for target in build.plates.into_iter().flatten() {
                backdrops.give(target);
            }
        }
        events.push(Event::BackdropFrozen {
            id,
            generation: set.ready.as_ref().map_or(0, |g| g.number),
        });
    }

    /// `BackdropRelease`: ends one lease.
    pub fn release(&mut self, id: BackdropId) {
        if let Some(set) = self.sets.get_mut(&id) {
            set.leases = set.leases.saturating_sub(1);
        }
    }

    /// Adds what every set holds to `census`: the plates of the generation shown and of the one being made, and the pictures.
    pub fn census(&self, census: &mut crate::census::Census) {
        for set in self.sets.values() {
            let held = set
                .ready
                .iter()
                .flat_map(|g| &g.plates)
                .chain(set.build.iter().flat_map(|b| &b.plates))
                .flatten();
            for plate in held {
                census.plates.texture(plate.width, plate.height, 4);
            }
            if let Some(picture) = &set.picture {
                census.pictures.texture(picture.width, picture.height, 4);
            }
        }
    }

    /// Whether glass over `id` has a picture to show.
    pub fn has_picture(&self, id: BackdropId) -> bool {
        self.sets
            .get(&id)
            .is_some_and(|set| set.ready.is_some() && set.picture.is_some())
    }

    /// The set a step is due for: one with a generation being made, else one that has been asked for a new one and is not
    /// held by a lease.
    pub fn due(&self) -> Option<BackdropId> {
        self.sets
            .iter()
            .find(|(_, set)| set.build.is_some())
            .or_else(|| {
                self.sets
                    .iter()
                    .find(|(_, set)| set.leases == 0 && set.dirty.is_some())
            })
            .map(|(id, _)| *id)
    }

    /// Starts a generation of `id` if none is being made (of the newest content asked for), and says what its next step is.
    pub fn step(&mut self, id: BackdropId) -> Option<Step> {
        let set = self.sets.get_mut(&id)?;
        if set.build.is_none() {
            let content = set.dirty.take()?;
            set.build = Some(Build {
                content,
                plates: Vec::with_capacity(set.plate_count()),
            });
        }
        let done = set.build.as_ref().map_or(0, |b| b.plates.len());
        let plate = match (set.spec.wallpaper, done) {
            (Some(wallpaper), 0) => Some(Plate::Wallpaper(wallpaper)),
            _ => set
                .spec
                .plates
                .get(done - set.wallpaper_offset())
                .map(|source| Plate::Group(source.group)),
        };
        Some(Step {
            id,
            plate,
            geometry: set.geometry,
            radius: set.spec.radius,
            last: done + 1 >= set.plate_count(),
        })
    }

    /// A step drew a plate (`None`: the plate has nothing in it).
    pub fn finish_plate(&mut self, id: BackdropId, plate: Option<RenderTarget>) {
        if let Some(build) = self.sets.get_mut(&id).and_then(|set| set.build.as_mut()) {
            build.plates.push(plate);
        }
    }

    /// Publishes the generation made: the set shows it from the next frame, and the one it replaces is given back. The
    /// picture the glass reads is made now if the set has none yet. `None` when the set is not there or was not being made.
    pub fn publish(
        &mut self,
        id: BackdropId,
        gl: &Gles,
        backdrops: &mut Backdrops,
    ) -> Option<Published> {
        let set = self.sets.get_mut(&id)?;
        let build = set.build.take()?;
        if set.picture.is_none() {
            let (w, h) = set.geometry.wallpaper_plate;
            set.picture = backdrops.take(gl, w, h);
            if set.picture.is_none() {
                log(format!(
                    "prepared backdrop {}: no room for a picture of {w}x{h}; its glass is drawn without one",
                    id.0
                ));
            }
        }
        let number = set.next;
        set.next += 1;
        let old = set.ready.replace(Generation {
            number,
            plates: build.plates,
        });
        for target in old.into_iter().flat_map(|g| g.plates).flatten() {
            backdrops.give(target);
        }
        set.composed = None;
        set.revision += 1;
        Some(Published {
            event: Event::BackdropReady {
                id,
                generation: number,
                content: build.content,
            },
            glass_frame: if set.picture.is_some() { set.used } else { 0 },
        })
    }

    /// What a frame needs to know of the set before it is drawn: glass is showing it, and the worlds of its groups are
    /// looked at (once a frame): if any changed while there is a generation to show, the picture is composed again.
    /// Returns the picture's revision.
    pub fn observe(
        &mut self,
        id: BackdropId,
        frame: u64,
        world_of: impl Fn(GroupId) -> Option<World>,
    ) -> u64 {
        let Some(set) = self.sets.get_mut(&id) else {
            return 0;
        };
        set.used = frame;
        if set.observed != frame {
            set.observed = frame;
            let mut changed = false;
            for (known, source) in set.worlds.iter_mut().zip(&set.spec.plates) {
                let world = world_of(source.group);
                changed |= *known != world;
                *known = world;
            }
            if changed && set.ready.is_some() {
                set.revision += 1;
            }
        }
        set.revision
    }

    /// What composing the picture of the set takes, when it is out of date; `None` when it is up to date or the set has
    /// nothing to show. [`Sets::composed`] says it is done.
    pub fn to_compose(&self, id: BackdropId) -> Option<Composition<'_>> {
        let set = self.sets.get(&id)?;
        let ready = set.ready.as_ref()?;
        let picture = set.picture?;
        let up_to_date = set
            .composed
            .as_ref()
            .is_some_and(|c| c.generation == ready.number && c.worlds == set.worlds);
        if up_to_date {
            return None;
        }
        Some(Composition {
            picture,
            geometry: set.geometry,
            wallpaper: set
                .spec
                .wallpaper
                .and_then(|_| ready.plates.first().copied().flatten()),
            groups: &ready.plates[set.wallpaper_offset()..],
            worlds: &set.worlds,
        })
    }

    /// The picture of `id` holds the generation and worlds it has now. (The worlds are copied into the vector the last
    /// composition kept, so that composing again as groups move allocates nothing.)
    pub fn composed(&mut self, id: BackdropId) {
        let Some(set) = self.sets.get_mut(&id) else {
            return;
        };
        let Some(ready) = &set.ready else {
            return;
        };
        match &mut set.composed {
            Some(composed) => {
                composed.generation = ready.number;
                composed.worlds.clone_from(&set.worlds);
            }
            None => {
                set.composed = Some(Composed {
                    generation: ready.number,
                    worlds: set.worlds.clone(),
                });
            }
        }
    }

    /// The texture the glass over `id` reads, if it has anything to show.
    pub fn picture(&self, id: BackdropId) -> Option<u32> {
        let set = self.sets.get(&id)?;
        set.ready.as_ref()?;
        set.picture.map(|p| p.texture)
    }

    /// The textures a set holds: the plates of the generation shown and of the one being made, and the picture (a test asks).
    #[cfg(test)]
    pub fn held(&self, id: BackdropId) -> usize {
        self.sets.get(&id).map_or(0, |set| {
            let plates = |plates: &[Option<RenderTarget>]| plates.iter().flatten().count();
            set.ready.as_ref().map_or(0, |g| plates(&g.plates))
                + set.build.as_ref().map_or(0, |b| plates(&b.plates))
                + usize::from(set.picture.is_some())
        })
    }

    /// How many sets there are (a test asks).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sets.len()
    }
}

/// What drawing the picture of a set takes.
pub struct Composition<'a> {
    pub picture: RenderTarget,
    pub geometry: Geometry,
    /// The wallpaper's plate, which is where the panel is.
    pub wallpaper: Option<RenderTarget>,
    /// The groups' plates and the worlds that place them, in the spec's order.
    pub groups: &'a [Option<RenderTarget>],
    pub worlds: &'a [Option<World>],
}

/// The logical position that the panel position (`px`, `py`) is of: [`Panel::panel_position`] undone.
pub fn logical_position(panel: &Panel, px: f32, py: f32) -> [f32; 2] {
    match panel.rotation {
        Rotation::R0 => [px, py],
        Rotation::R90 => [panel.logical_width - py, px],
        Rotation::R180 => [panel.logical_width - px, panel.logical_height - py],
        Rotation::R270 => [py, panel.logical_height - px],
    }
}

/// The corners, as a triangle strip, of a group's plate of `size` texels as the group's `world` places it on the panel:
/// each corner's panel position and the place in the plate, in texels, that it shows. The plate's first texel is at
/// (`-margin`, `-margin`) of the group's own space in panel pixels (the margin round the panel) and a texel is `texel`
/// pixels. The group's space is the screen's, turned the way the panel is, so each corner is turned back to the
/// screen's orientation, placed by the world's similarity, and turned onto the panel again. Without a world the
/// plate is where the panel's own pixels are.
pub fn plate_quad(
    panel: &Panel,
    margin: u32,
    texel: u32,
    size: (u32, u32),
    world: Option<&World>,
) -> [Vertex; 4] {
    let corner = |u: u32, v: u32| -> Vertex {
        let px = u as f32 * texel as f32 - margin as f32;
        let py = v as f32 * texel as f32 - margin as f32;
        let [x, y] = logical_position(panel, px, py);
        let (x, y) = world.map_or((x, y), |w| w.transform.apply(x, y));
        let [qx, qy] = panel.panel_position(x, y);
        [qx, qy, u as f32, v as f32]
    };
    [
        corner(0, 0),
        corner(size.0, 0),
        corner(0, size.1),
        corner(size.0, size.1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::group::Transform;

    fn panel(rotation: Rotation) -> Panel {
        let swaps = rotation.swaps_axes();
        Panel {
            rotation,
            logical_width: 960.0,
            logical_height: 480.0,
            width: if swaps { 480 } else { 960 },
            height: if swaps { 960 } else { 480 },
        }
    }

    const ROTATIONS: [Rotation; 4] = [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270];

    #[test]
    fn the_panel_position_of_a_logical_one_turns_back_to_it_for_every_rotation() {
        for rotation in ROTATIONS {
            let p = panel(rotation);
            for (x, y) in [(0.0, 0.0), (960.0, 480.0), (123.5, 77.25), (-40.0, 600.0)] {
                let [px, py] = p.panel_position(x, y);
                assert_eq!(logical_position(&p, px, py), [x, y], "{rotation:?}");
            }
        }
    }

    fn world(scale: f32, tx: f32, ty: f32) -> World {
        World {
            transform: Transform { scale, tx, ty },
            opacity: 1.0,
            visible: true,
            clip: None,
            feather: compositor_protocol::Edges::NONE,
        }
    }

    #[test]
    fn a_plate_covers_its_texels_on_the_panel_and_a_moved_one_goes_with_its_group_on_the_screen() {
        // A plate of 20 x 40 texels of 8 pixels, with a margin of 16 pixels all round: 160 x 320 pixels of the panel, from
        // 16 before the panel's own origin, in whichever way the panel is turned (the plate is a picture of the panel).
        for rotation in ROTATIONS {
            let p = panel(rotation);
            let rest = plate_quad(&p, 16, 8, (20, 40), None);
            assert_eq!(rest[0], [-16.0, -16.0, 0.0, 0.0], "{rotation:?}");
            assert_eq!(rest[3], [144.0, 304.0, 20.0, 40.0], "{rotation:?}");
            let identity = plate_quad(&p, 16, 8, (20, 40), Some(&world(1.0, 0.0, 0.0)));
            assert_eq!(rest, identity, "{rotation:?}");
            // A group moved 100 right and 30 down on the screen moves its plate by that much on the screen, which is
            // wherever it is on the panel, and no texel of the plate changes.
            let moved = plate_quad(&p, 16, 8, (20, 40), Some(&world(1.0, 100.0, 30.0)));
            for (a, b) in rest.iter().zip(&moved) {
                let (ra, rb) = (
                    logical_position(&p, a[0], a[1]),
                    logical_position(&p, b[0], b[1]),
                );
                assert_eq!(
                    [rb[0] - ra[0], rb[1] - ra[1]],
                    [100.0, 30.0],
                    "{rotation:?}"
                );
                assert_eq!(a[2..], b[2..], "{rotation:?}");
            }
        }
    }

    #[test]
    fn a_scaled_group_scales_its_plate_about_the_screens_origin_as_its_similarity_says() {
        let p = panel(Rotation::R0);
        let scaled = plate_quad(&p, 0, 8, (10, 10), Some(&world(0.5, 40.0, 0.0)));
        // Texel (0, 0) is at (0, 0) of the group's space, which the world puts at (40, 0); 80 pixels of plate are 40 on screen.
        assert_eq!(scaled[0], [40.0, 0.0, 0.0, 0.0]);
        assert_eq!(scaled[3], [80.0, 40.0, 10.0, 10.0]);
    }
}
