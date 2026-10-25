//! Still glass (`ideas::STILL_GLASS`): a glass surface that stands as it stood, with nothing behind it changing but wallpapers,
//! is drawn once into a *plate*, a texture of the panel's size that holds the surface alone (over nothing), and shown from the
//! plate for the frames that follow: a copy of its pixels where its rim's refraction and edge would be worked out again, and a
//! picture of what is behind it made again, every frame. (A plate is the compositor's own texture; the display's planes, which
//! the overlay route draws into, are another thing: `scanout/plan.rs`.)
//!
//! What is decided here is only which surfaces are shown from which plate and when their plate is made again; the drawing is the
//! renderer's. The surfaces that share a picture (one backdrop, `backdrop::Backdrop::users`) are one group and are decided
//! together, so that the picture, which they all read, is made in the same frame for all of them or in none.
//!
//! * A group is drawn as it always was in a frame that follows one in which it differed (something moved, was restyled, or
//!   changed below), and for `STANDS - 1` frames more: motion is not worth a plate, and neither is a picture that changes every
//!   second or third frame (a video tile below the glass), which would pay for plates that are thrown away the frame after.
//! * The next frame makes the plates (the surfaces are drawn into them and then shown from them); the ones after show them.
//! * A live picture of moving wallpapers is not what the plate holds for long: it is made again after a few frames, staggered
//!   by the group's leader so that the pictures of a screen are not all made in one frame. What a cached picture is of does not
//!   move, so the plate stands until the picture is made again.
//! * Surfaces whose bodies overlap (a tile on a sheet) are in different plates, and a surface that finds no place is drawn as
//!   it always was.

use crate::backdrop::{Area, Refresh, overlaps};
use crate::scene::Drawn;
use compositor_protocol::{Content, LayerId};
use std::collections::HashMap;

/// How many plates there are: a surface that overlaps others is in a plate where nothing does.
pub const PLATES: usize = 3;

/// A live picture in a plate is made again after this many frames, and up to `SPREAD - 1` more, by group.
const PERIOD: u64 = 3;
const SPREAD: u64 = 3;

/// Frames in a row a group must stand as the frame before had it before its plates are made. Two made intercom's glass, over a
/// tile that changes every second frame, pay for plates in half its frames and miss a refresh at T3 (RESULTS, 09-30); three
/// also passes over what changes every third frame, at the cost of two direct frames more when something comes to rest.
const STANDS: u32 = 3;

/// A group that has not been looked at for this many frames is forgotten, and its places in the plates are free.
const KEEP_FRAMES: u64 = 600;

/// One picture's surfaces, as the frame has them.
pub struct Group {
    /// The first surface's layer: what the group is known by.
    pub leader: LayerId,
    /// The surfaces, by layer index.
    pub users: Vec<usize>,
    pub refresh: Refresh,
    pub radius: f32,
    pub downsample: u8,
    /// What a cached picture is a picture of (`backdrop::fingerprint`); 0 for a live one.
    pub fingerprint: u64,
    /// A cached picture that is out of date: it is made again in this frame.
    pub stale: bool,
    /// A live picture's other contents: the layers below that show in it, a wallpaper's own motion left out.
    pub below: Vec<Drawn>,
    /// Whether the frame draws any of the group's surfaces.
    pub drawn: bool,
}

/// Everything that the plates hold of a group depends on: two frames with the same one draw the group the same.
#[derive(Clone, PartialEq)]
struct Key {
    users: Vec<Drawn>,
    below: Vec<Drawn>,
    refresh: Refresh,
    radius: f32,
    downsample: u8,
    fingerprint: u64,
}

/// Where one surface of a made group is: its plate and the panel pixels it covers there.
#[derive(Clone, Copy, Debug)]
struct Member {
    plate: usize,
    rect: Area,
}

struct Made {
    key: Key,
    members: Vec<Member>,
    /// The frame that drew the group into its plates.
    frame: u64,
}

#[derive(Default)]
struct Watch {
    /// The group as the last frame that drew it had it: a group that differs is in motion.
    last: Option<Key>,
    made: Option<Made>,
    /// The last frame that looked at the group.
    seen: u64,
    /// Frames in a row that the group has stood as the one before had it.
    stands: u32,
}

/// How a surface is shown in this frame: from its plate, which is made again first when `render`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reuse {
    pub plate: usize,
    /// The panel pixels of the plate the surface is in (its body, and the fringe of its edge).
    pub rect: Area,
    pub render: bool,
}

pub struct Plan {
    /// By layer index: the surfaces shown from a plate; the rest are drawn as they always were.
    pub reuse: Vec<Option<Reuse>>,
    /// By group: whether the frame has to make its picture. It does not for a group shown from plates that stand.
    pub picture: Vec<bool>,
}

impl Plan {
    /// Every surface drawn as it always was, every picture made.
    pub fn direct(layers: usize, groups: usize) -> Self {
        Self {
            reuse: vec![None; layers],
            picture: vec![true; groups],
        }
    }
}

#[derive(Default)]
pub struct Still {
    watch: HashMap<LayerId, Watch>,
}

/// A group that is to be drawn into plates now.
struct Fresh {
    leader: LayerId,
    users: Vec<usize>,
    key: Key,
    rects: Vec<Area>,
}

/// The plates for `rects` in turn, each a plate where none of `occupied` (a plate and the pixels taken in it) and none of the
/// rects before overlaps it; `None` when one finds no place.
fn place(occupied: &[(usize, Area)], rects: &[Area]) -> Option<Vec<usize>> {
    let mut taken = occupied.to_vec();
    let mut plates = Vec::with_capacity(rects.len());
    for rect in rects {
        let plate =
            (0..PLATES).find(|&p| !taken.iter().any(|(q, r)| *q == p && overlaps(r, rect)))?;
        taken.push((plate, *rect));
        plates.push(plate);
    }
    Some(plates)
}

impl Still {
    /// Decides how frame number `frame` shows `groups` (`layers` are as the frame draws them, `rects` the panel pixels of each
    /// surface by layer index, `None` where it is nowhere on the panel).
    pub fn classify(
        &mut self,
        frame: u64,
        layers: &[Drawn],
        groups: Vec<Group>,
        rects: &[Option<Area>],
    ) -> Plan {
        let mut plan = Plan::direct(layers.len(), groups.len());
        self.watch.retain(|id, w| {
            frame.saturating_sub(w.seen) <= KEEP_FRAMES && layers.iter().any(|l| l.id == *id)
        });
        // First the groups that stand as their plates have them, so that what is left of the plates is known.
        let mut fresh = Vec::new();
        for (g, group) in groups.into_iter().enumerate().filter(|(_, g)| g.drawn) {
            let cached = group.refresh == Refresh::Cached;
            let key = Key {
                users: group.users.iter().map(|&u| layers[u]).collect(),
                below: group.below,
                refresh: group.refresh,
                radius: group.radius,
                downsample: group.downsample,
                fingerprint: group.fingerprint,
            };
            let watch = self.watch.entry(group.leader).or_default();
            watch.seen = frame;
            let stood = watch.last.as_ref() == Some(&key);
            let holds = stood
                && watch
                    .made
                    .as_ref()
                    .is_some_and(|m| m.key == key && !(cached && group.stale));
            watch.last = Some(key.clone());
            watch.stands = if stood {
                watch.stands.saturating_add(1)
            } else {
                0
            };
            let rects: Option<Vec<Area>> = group.users.iter().map(|&u| rects[u]).collect();
            let Some(rects) = rects.filter(|_| stood) else {
                watch.made = None;
                continue;
            };
            let Some(made) = watch.made.as_mut().filter(|_| holds) else {
                watch.made = None;
                if watch.stands < STANDS {
                    continue;
                }
                fresh.push(Fresh {
                    leader: group.leader,
                    users: group.users,
                    key,
                    rects,
                });
                continue;
            };
            // A live picture is made again every few frames only when a wallpaper is in it: a wallpaper's own motion is left out of
            // the key (`renderer::under_of`), and it is what the picture then goes stale by. What else is in the picture is in the key
            // (a layer that moved, was restyled or was damaged is another key, and the plate is made again at once), so a picture of
            // nothing but still layers (the shell's own wallpaper is a picture, not a wallpaper layer) stands for as long as they do.
            let moving = key
                .below
                .iter()
                .any(|l| matches!(l.content, Content::Wallpaper(_)));
            let period = PERIOD + u64::from(group.leader.0) % SPREAD;
            let due = !cached && moving && frame.saturating_sub(made.frame) >= period;
            if due {
                made.frame = frame;
            } else {
                plan.picture[g] = false;
            }
            for (&user, member) in group.users.iter().zip(&made.members) {
                plan.reuse[user] = Some(Reuse {
                    plate: member.plate,
                    rect: member.rect,
                    render: due,
                });
            }
        }
        // Then the groups that are drawn into plates now, each where the ones before left room.
        for Fresh {
            leader,
            users,
            key,
            rects,
        } in fresh
        {
            let occupied: Vec<(usize, Area)> = self
                .watch
                .values()
                .filter_map(|w| w.made.as_ref())
                .flat_map(|made| made.members.iter().map(|m| (m.plate, m.rect)))
                .collect();
            let Some(plates) = place(&occupied, &rects) else {
                continue;
            };
            let members: Vec<Member> = plates
                .into_iter()
                .zip(rects)
                .map(|(plate, rect)| Member { plate, rect })
                .collect();
            for (&user, member) in users.iter().zip(&members) {
                plan.reuse[user] = Some(Reuse {
                    plate: member.plate,
                    rect: member.rect,
                    render: true,
                });
            }
            self.watch.entry(leader).or_default().made = Some(Made {
                key,
                members,
                frame,
            });
        }
        plan
    }

    /// Lets go of what stands in `plate`: its texture is gone, so what was drawn into it for a group cannot be shown, and the group
    /// is made again once it has stood.
    pub fn drop_plate(&mut self, plate: usize) {
        for watch in self.watch.values_mut() {
            if watch
                .made
                .as_ref()
                .is_some_and(|made| made.members.iter().any(|m| m.plate == plate))
            {
                watch.made = None;
            }
        }
    }

    /// Lets go of a group's plates: what was drawn into them for it cannot be shown (its picture could not be made).
    pub fn forget(&mut self, leader: LayerId) {
        if let Some(watch) = self.watch.get_mut(&leader) {
            watch.made = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgl::drawn;
    use compositor_protocol::{Content, Props};

    fn layer(id: u32, x: f32) -> Drawn {
        drawn(id, (200, 100), Content::Image, Props::at(x, 50.0))
    }

    fn rect(x: i32) -> Option<Area> {
        Some([x, 50, x + 200, 150])
    }

    fn group(leader: u32, users: &[usize], refresh: Refresh) -> Group {
        Group {
            leader: LayerId(leader),
            users: users.to_vec(),
            refresh,
            radius: 30.0,
            downsample: 4,
            fingerprint: 7,
            stale: false,
            below: Vec::new(),
            drawn: true,
        }
    }

    fn shown(plan: &Plan, layer: usize) -> Option<bool> {
        plan.reuse[layer].map(|r| r.render)
    }

    /// A wallpaper layer, as a live picture holds one below its glass (its own motion is left out of the key).
    fn wallpaper() -> Drawn {
        drawn(
            9,
            (960, 480),
            Content::Wallpaper(compositor_protocol::Wallpaper {
                kind: compositor_protocol::WallpaperKind::Starfield,
                speed: 1.0,
                intensity: 0.7,
                fps: 60,
            }),
            Props::at(0.0, 0.0),
        )
    }

    #[test]
    fn a_group_that_stands_is_drawn_as_always_until_it_has_stood_long_enough_then_made_into_plates_and_shown_from_them()
     {
        let mut still = Still::default();
        let layers = [layer(1, 0.0)];
        let rects = [rect(0)];
        let mut frame =
            |n| still.classify(n, &layers, vec![group(1, &[0], Refresh::Cached)], &rects);
        for n in 1..=u64::from(STANDS) {
            let direct = frame(n);
            assert_eq!(
                (shown(&direct, 0), direct.picture[0]),
                (None, true),
                "frame {n}"
            );
        }
        let made = frame(u64::from(STANDS) + 1);
        assert_eq!((shown(&made, 0), made.picture[0]), (Some(true), true));
        assert_eq!(made.reuse[0].unwrap().rect, rect(0).unwrap());
        for n in u64::from(STANDS) + 2..60 {
            let plan = frame(n);
            assert_eq!(
                (shown(&plan, 0), plan.picture[0]),
                (Some(false), false),
                "a cached picture's plate stands, whatever the frame"
            );
        }
    }

    #[test]
    fn a_live_picture_with_a_wallpaper_in_it_is_made_again_after_its_period_and_the_groups_of_a_screen_are_not_in_step()
     {
        let mut still = Still::default();
        let layers = [layer(1, 0.0), layer(2, 300.0)];
        let rects = [rect(0), rect(300)];
        let mut made_at: [Vec<u64>; 2] = Default::default();
        for n in 1..=30 {
            let mut groups = vec![group(1, &[0], Refresh::Live), group(2, &[1], Refresh::Live)];
            for g in &mut groups {
                g.below = vec![wallpaper()];
            }
            let plan = still.classify(n, &layers, groups, &rects);
            for (g, at) in made_at.iter_mut().enumerate() {
                if plan.picture[g] && plan.reuse[g].is_some() {
                    at.push(n);
                }
            }
        }
        // Group 1 has a period of 4, group 2 one of 5: made in frame 4 (after standing 3), and again every period.
        assert_eq!(made_at[0], [4, 8, 12, 16, 20, 24, 28]);
        assert_eq!(made_at[1], [4, 9, 14, 19, 24, 29]);
    }

    /// What no wallpaper is in cannot go stale by itself: everything that is in the picture is in the key, and a layer that moves,
    /// is restyled or is damaged (a new version) is another key. The shell's own wallpaper is a picture (an image layer), so the
    /// home's glass over it is made into its plate once and shown from it for as long as nothing below changes: the blur is not made
    /// again every few frames (4.8 ms of the sheet's frames while it moves over the home).
    #[test]
    fn a_live_picture_of_still_layers_stands_until_something_in_it_changes() {
        let mut still = Still::default();
        let layers = [layer(1, 0.0)];
        let rects = [rect(0)];
        let ground = layer(7, 0.0);
        let mut frame = |n: u64, below: Drawn| {
            let mut g = group(1, &[0], Refresh::Live);
            g.below = vec![below];
            still.classify(n, &layers, vec![g], &rects)
        };
        let mut made = Vec::new();
        for n in 1..=60 {
            let plan = frame(n, ground);
            if plan.picture[0] && plan.reuse[0].is_some() {
                made.push(n);
            }
        }
        assert_eq!(made, [u64::from(STANDS) + 1], "made once, never again");
        // A change below (the picture is damaged: a new version) is a change of the group: drawn as always, then made again.
        let changed = Drawn {
            version: ground.version + 1,
            ..ground
        };
        assert_eq!(shown(&frame(61, changed), 0), None);
        let again: Vec<u64> = (62..=80)
            .filter(|&n| {
                let plan = frame(n, changed);
                plan.picture[0] && plan.reuse[0].is_some()
            })
            .collect();
        assert_eq!(again, [61 + u64::from(STANDS)], "made again after standing");
    }

    #[test]
    fn a_change_draws_the_group_as_always_and_the_frame_after_it_makes_the_plates_again() {
        let mut still = Still::default();
        let rects = [rect(0)];
        let mut frame = |n: u64, x: f32| {
            let layers = [layer(1, x)];
            still.classify(n, &layers, vec![group(1, &[0], Refresh::Cached)], &rects)
        };
        for n in 1..=4 {
            frame(n, 0.0);
        }
        assert_eq!(shown(&frame(5, 0.0), 0), Some(false));
        // It moves, every frame: never from a plate.
        for (n, x) in (6..14).zip(1..) {
            assert_eq!(shown(&frame(n, x as f32), 0), None, "frame {n}");
        }
        // At rest at 8 from frame 13: two frames more drawn as always, then the plates.
        assert_eq!(shown(&frame(14, 8.0), 0), None);
        assert_eq!(shown(&frame(15, 8.0), 0), None);
        assert_eq!(shown(&frame(16, 8.0), 0), Some(true));
        assert_eq!(shown(&frame(17, 8.0), 0), Some(false));
    }

    #[test]
    fn what_changes_below_a_live_picture_is_a_change_of_the_group() {
        let mut still = Still::default();
        let layers = [layer(1, 0.0)];
        let rects = [rect(0)];
        let mut frame = |n: u64, below: Drawn| {
            let mut g = group(1, &[0], Refresh::Live);
            g.below = vec![below];
            still.classify(n, &layers, vec![g], &rects)
        };
        let under = layer(9, 0.0);
        for n in 1..=3 {
            frame(n, under);
        }
        assert_eq!(shown(&frame(4, under), 0), Some(true));
        assert_eq!(shown(&frame(5, under), 0), Some(false));
        let moved = layer(9, 5.0);
        assert_eq!(shown(&frame(6, moved), 0), None, "it moved below");
        assert_eq!(shown(&frame(7, moved), 0), None);
        assert_eq!(shown(&frame(8, moved), 0), None);
        assert_eq!(shown(&frame(9, moved), 0), Some(true));
        let mut restyled = moved;
        restyled.version += 1;
        assert_eq!(shown(&frame(10, restyled), 0), None, "it changed below");
    }

    #[test]
    fn a_cached_picture_that_is_made_again_is_a_change_of_what_the_plates_hold() {
        let mut still = Still::default();
        let layers = [layer(1, 0.0)];
        let rects = [rect(0)];
        let mut frame = |n: u64, fingerprint: u64, stale: bool| {
            let mut g = group(1, &[0], Refresh::Cached);
            (g.fingerprint, g.stale) = (fingerprint, stale);
            still.classify(n, &layers, vec![g], &rects)
        };
        for n in 1..=4 {
            frame(n, 7, false);
        }
        assert_eq!(shown(&frame(5, 7, false), 0), Some(false));
        // Made again this frame: what the plates hold is out of date, and is drawn again from the new picture once it has stood.
        assert_eq!(shown(&frame(6, 8, true), 0), None, "what it is of changed");
        assert_eq!(shown(&frame(7, 8, false), 0), None);
        assert_eq!(shown(&frame(8, 8, false), 0), None);
        assert_eq!(shown(&frame(9, 8, false), 0), Some(true));
        assert_eq!(shown(&frame(10, 8, false), 0), Some(false));
        // The same fingerprint with the picture gone (stale): it has stood all along, so it is drawn into the plates again at once.
        assert_eq!(shown(&frame(11, 8, true), 0), Some(true));
    }

    #[test]
    fn surfaces_that_overlap_are_in_different_plates_and_one_that_finds_no_place_is_drawn_as_always()
     {
        let mut still = Still::default();
        // Five surfaces on top of one another: three plates have room for three.
        let layers: Vec<Drawn> = (1..=5).map(|id| layer(id, 0.0)).collect();
        let rects: Vec<Option<Area>> = (1..=5).map(|_| rect(0)).collect();
        let mut frame = |n: u64| {
            let groups = (0..5)
                .map(|g| group(g as u32 + 1, &[g], Refresh::Cached))
                .collect();
            still.classify(n, &layers, groups, &rects)
        };
        for n in 1..=3 {
            frame(n);
        }
        let plan = frame(4);
        let plates: Vec<Option<usize>> = plan.reuse.iter().map(|r| r.map(|r| r.plate)).collect();
        assert_eq!(plates, [Some(0), Some(1), Some(2), None, None]);
        // Side by side: all in the first plate.
        let apart: Vec<Option<Area>> = (0..5).map(|k| rect(k * 300)).collect();
        let mut still = Still::default();
        let mut frame = |n: u64| {
            let groups = (0..5)
                .map(|g| group(g as u32 + 1, &[g], Refresh::Cached))
                .collect();
            still.classify(n, &layers, groups, &apart)
        };
        for n in 1..=3 {
            frame(n);
        }
        let plan = frame(4);
        assert!(plan.reuse.iter().all(|r| r.is_some_and(|r| r.plate == 0)));
    }

    #[test]
    fn a_group_that_is_not_drawn_keeps_its_place_and_shows_from_it_when_it_is_drawn_again() {
        let mut still = Still::default();
        let layers = [layer(1, 0.0), layer(2, 0.0)];
        let rects = [rect(0), rect(0)];
        let mut frame = |n: u64, first: bool| {
            let mut a = group(1, &[0], Refresh::Cached);
            a.drawn = first;
            still.classify(n, &layers, vec![a, group(2, &[1], Refresh::Cached)], &rects)
        };
        for n in 1..=3 {
            frame(n, true);
        }
        let plan = frame(4, true);
        assert_eq!(plan.reuse[0].unwrap().plate, 0);
        assert_eq!(
            plan.reuse[1].unwrap().plate,
            1,
            "the second is over the first"
        );
        // The first is not drawn for a while: the second stays in the plate it is in.
        for n in 5..10 {
            assert_eq!(frame(n, false).reuse[1].unwrap().plate, 1);
        }
        // It comes back as it was, from its plate, without being made again.
        assert_eq!(shown(&frame(10, true), 0), Some(false));
    }

    #[test]
    fn a_layer_that_is_gone_lets_go_of_its_place() {
        let mut still = Still::default();
        let (a, b, c) = (layer(1, 0.0), layer(2, 0.0), layer(3, 0.0));
        let cached = |leader| group(leader, &[0], Refresh::Cached);
        for n in 1..=4 {
            still.classify(
                n,
                &[a, b],
                vec![cached(1), group(2, &[1], Refresh::Cached)],
                &[rect(0), rect(0)],
            );
        }
        // The first goes and a new one, over the second, comes: it finds the first's place free once it has stood.
        for n in 5..=7 {
            still.classify(
                n,
                &[b, c],
                vec![cached(2), group(3, &[1], Refresh::Cached)],
                &[rect(0), rect(0)],
            );
        }
        let plan = still.classify(
            8,
            &[b, c],
            vec![cached(2), group(3, &[1], Refresh::Cached)],
            &[rect(0), rect(0)],
        );
        assert_eq!(
            plan.reuse[0].unwrap().plate,
            1,
            "the one that stays is where it was"
        );
        assert_eq!(plan.reuse[1].unwrap().plate, 0);
    }

    #[test]
    fn glass_over_a_picture_that_changes_every_second_or_third_frame_never_pays_for_plates() {
        for every in [2u64, 3] {
            let mut still = Still::default();
            let layers = [layer(1, 0.0)];
            let rects = [rect(0)];
            for n in 1..60 {
                // A video tile below: a new picture every `every` frames.
                let mut below = layer(9, 0.0);
                below.version = n / every;
                let mut g = group(1, &[0], Refresh::Live);
                g.below = vec![below];
                let plan = still.classify(n, &layers, vec![g], &rects);
                assert_eq!(
                    shown(&plan, 0),
                    None,
                    "a change every {every} frames, frame {n}"
                );
            }
        }
    }
}
