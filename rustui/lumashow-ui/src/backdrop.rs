//! Prepared backdrops, as the app holds them (design section 3.2): a small bundle of blurred plates of retained groups and
//! the wallpaper behind them, made when the app *asks* and never because a surface moved, was touched or drew.
//!
//! [`Scene::prepare_backdrop`] names what the set captures. [`PreparedBackdrop::request_refresh`] says the content changed (the
//! compositor makes the next generation in a moment it has to spare, and answers with `Event::BackdropReady`, which
//! [`Scene::observe`] folds into [`PreparedBackdrop::ready`]). A glass surface made with
//! `Backdrop::Prepared { id: backdrop.wire_id() }` shows the newest finished generation, or the one a [`BackdropLease`] holds:
//! the app freezes the ready generation for the lifetime of a focus or shade gesture, so that nothing is captured meanwhile, and
//! lets the lease go when the content has settled.
//!
//! The set captures groups and a layer, so they outlive it: dropping the set deletes it on the compositor (its captures
//! cancelled, its leases ended) *before* the groups it captured, whose own deletion waits for the set's (`group.rs`), and the
//! wallpaper layer's deletion waits too (`Scene::delete_layer`).

use crate::error::{Kind, Limit, UiError};
use crate::group::GroupId;
use crate::handle::handle;
use crate::scene::{Inner, Scene};
use crate::writer::Urgent;
use compositor_protocol as wire;
use compositor_protocol::resources::{MAX_BACKDROPS, MAX_PLATES};
use compositor_protocol::{CaptureSpec, LayerId, PlateSource, Request};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

handle!(
    /// Names a prepared set in this process: an index and a generation, never reused. The compositor's id for it is
    /// [`PreparedBackdrop::wire_id`].
    BackdropId
);

/// An app-chosen number for the content a prepared set shows (theme, wallpaper, face, tile and card content, not touch
/// transforms): it comes back in `Event::BackdropReady`, so the app knows which of its changes a generation includes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentGeneration(pub u64);

/// A finished generation of a prepared set, numbered by the compositor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackdropGeneration(pub u64);

/// What a prepared set is made of: the design's bundle of group plates and the wallpaper plate (`CaptureSpec`, with the app's
/// own handles in place of wire ids).
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropSpec {
    /// The wallpaper layer drawn under every plate, as a plate of its own; `None` leaves the set transparent there.
    pub wallpaper: Option<LayerId>,
    /// At most 8 groups, drawn in this order (first is the bottom), each as its own plate. A plate keeps the transform of its
    /// group, so the dock and the content move in lockstep over their own plates with no new capture.
    pub plates: Vec<GroupId>,
    /// The blur's standard deviation in logical pixels, 0 to 64 (Lucent's `blur.surface` is 30).
    pub radius: f32,
    /// How many times smaller than the screen the plates are: 2, 4 or 8.
    pub downsample: u8,
}

/// Where a lease is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseState {
    /// The request is out; the compositor has not said which generation it holds.
    Pending,
    Held(BackdropGeneration),
    /// The set was deleted (which ends its leases) or the lease was dropped.
    Over,
}

pub(crate) struct BackdropEntry {
    pub(crate) wire: wire::BackdropId,
    /// The commit that creates the set: requests about it wait for that commit to have been written.
    created_tag: u64,
    wallpaper: Option<LayerId>,
    plates: Vec<GroupId>,
    /// The newest finished generation and the content it was made for.
    ready: Option<(BackdropGeneration, ContentGeneration)>,
    /// Leases asked for whose answers (`Event::BackdropFrozen`) have not come, oldest first: the compositor answers in order.
    pending: VecDeque<Rc<Cell<LeaseState>>>,
    /// Leases held, so that deleting the set can end them.
    held: Vec<Rc<Cell<LeaseState>>>,
}

/// Urgent requests about sets share the stream numbers with tracks' by this flag, so that no two kinds of thing are one stream.
const BACKDROP_STREAM: u32 = 0x8000_0000;

impl Inner {
    pub(crate) fn backdrop_create(&mut self, spec: &BackdropSpec) -> Result<BackdropId, UiError> {
        if spec.plates.len() > MAX_PLATES {
            return Err(UiError::Limit(Limit::Plates));
        }
        if spec.plates.is_empty() && spec.wallpaper.is_none() {
            return Err(UiError::Invalid(
                "a prepared backdrop captures a wallpaper or a group",
            ));
        }
        if !spec.radius.is_finite() || !(0.0..=64.0).contains(&spec.radius) {
            return Err(UiError::Invalid(
                "the blur radius of a prepared backdrop is 0 to 64",
            ));
        }
        if !matches!(spec.downsample, 2 | 4 | 8) {
            return Err(UiError::Invalid(
                "a prepared backdrop's plates are 2, 4 or 8 times smaller than the screen",
            ));
        }
        for (i, plate) in spec.plates.iter().enumerate() {
            if spec.plates[..i].contains(plate) {
                return Err(UiError::Invalid(
                    "a group is one plate of a prepared backdrop at most",
                ));
            }
            if self.groups.get(plate.key()).is_none() {
                return Err(UiError::Stale(Kind::Group));
            }
        }
        if self.backdrops.is_full() {
            return Err(UiError::Limit(Limit::Backdrops));
        }
        let mut plates = Vec::with_capacity(spec.plates.len());
        for plate in &spec.plates {
            plates.push(PlateSource {
                group: self.group_pin(*plate)?,
            });
        }
        if let Some(layer) = spec.wallpaper {
            self.pin_layer(layer);
        }
        let wire = wire::BackdropId(self.next_backdrop);
        self.next_backdrop += 1;
        let entry = BackdropEntry {
            wire,
            created_tag: self.next_tag,
            wallpaper: spec.wallpaper,
            plates: spec.plates.clone(),
            ready: None,
            pending: VecDeque::new(),
            held: Vec::new(),
        };
        let key = self
            .backdrops
            .insert(entry)
            .expect("room was checked above");
        self.batch.push(Request::BackdropCreate {
            id: wire,
            spec: CaptureSpec {
                wallpaper: spec.wallpaper,
                plates,
                radius: spec.radius,
                downsample: spec.downsample,
            },
        });
        Ok(BackdropId::from_key(key))
    }

    /// A set is dropped: it is deleted (which ends its leases), then what it captured is let go.
    fn backdrop_release(&mut self, id: BackdropId) {
        let Some(entry) = self.backdrops.remove(id.key()) else {
            return;
        };
        for lease in entry.pending.iter().chain(&entry.held) {
            lease.set(LeaseState::Over);
        }
        self.out.forget_stream(BACKDROP_STREAM | entry.wire.0);
        self.batch.push(Request::BackdropDelete { id: entry.wire });
        for plate in entry.plates {
            self.release_member(plate);
        }
        if let Some(layer) = entry.wallpaper {
            self.unpin_layer(layer);
        }
    }

    pub(crate) fn observe_backdrop_ready(
        &mut self,
        id: wire::BackdropId,
        generation: u64,
        content: u64,
    ) {
        if let Some((_, _, entry)) = self.backdrops.iter_mut().find(|(_, _, b)| b.wire == id) {
            if entry.ready.is_none_or(|(g, _)| generation > g.0) {
                entry.ready = Some((BackdropGeneration(generation), ContentGeneration(content)));
            }
        }
    }

    pub(crate) fn observe_backdrop_frozen(&mut self, id: wire::BackdropId, generation: u64) {
        let Some((_, _, entry)) = self.backdrops.iter_mut().find(|(_, _, b)| b.wire == id) else {
            return;
        };
        let Some(lease) = entry.pending.pop_front() else {
            return;
        };
        // A lease that was dropped before the answer came has nothing to be told.
        if Rc::strong_count(&lease) > 1 {
            lease.set(LeaseState::Held(BackdropGeneration(generation)));
            entry.held.push(lease);
        }
    }
}

/// A prepared set of blurred plates. Dropping it deletes it on the compositor and ends its leases.
pub struct PreparedBackdrop {
    id: BackdropId,
    scene: Scene,
}

impl Scene {
    /// A prepared backdrop of `spec`. At most 4 at once, 8 plates each; the groups of the plates must be alive. Nothing is made
    /// until the first [`PreparedBackdrop::request_refresh`].
    pub fn prepare_backdrop(&self, spec: &BackdropSpec) -> Result<PreparedBackdrop, UiError> {
        let id = self.inner.borrow_mut().backdrop_create(spec)?;
        Ok(PreparedBackdrop {
            id,
            scene: self.clone(),
        })
    }
}

impl PreparedBackdrop {
    pub fn id(&self) -> BackdropId {
        self.id
    }

    fn with<R>(&self, f: impl FnOnce(&BackdropEntry) -> R) -> R {
        f(self
            .scene
            .inner
            .borrow()
            .backdrops
            .get(self.id.key())
            .expect("a live handle"))
    }

    /// The compositor's name for the set: what `Backdrop::Prepared { id }` of a glass layer names.
    pub fn wire_id(&self) -> wire::BackdropId {
        self.with(|b| b.wire)
    }

    /// The content has changed to `content`: the compositor makes the next generation in a moment it has to spare (unless a
    /// lease holds the set) and answers when it is ready. Staged with the transaction that changes the content, so the
    /// request and the change are one. Only the newest content is made: a request that is still staged is replaced.
    pub fn request_refresh(&self, content: ContentGeneration) {
        let mut guard = self.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let wire = inner
            .backdrops
            .get(self.id.key())
            .expect("a live handle")
            .wire;
        for request in inner.batch.iter_mut().rev() {
            if let Request::BackdropRefresh {
                id,
                content: staged,
            } = request
            {
                if *id == wire {
                    *staged = content.0;
                    return;
                }
            }
        }
        inner.batch.push(Request::BackdropRefresh {
            id: wire,
            content: content.0,
        });
    }

    /// The newest finished generation, once the compositor has said there is one.
    pub fn ready(&self) -> Option<BackdropGeneration> {
        self.with(|b| b.ready.map(|(g, _)| g))
    }

    /// The content the newest finished generation was made for.
    pub fn ready_content(&self) -> Option<ContentGeneration> {
        self.with(|b| b.ready.map(|(_, c)| c))
    }

    /// Holds the generation the glass shows now until the lease is dropped: no capture of the set is made, and the set is not
    /// replaced, meanwhile; a refresh that comes during it is made when the last lease ends. The request goes out at once. The
    /// compositor holds the generation it shows when the request reaches it, which is `generation` or a newer one that
    /// finished meanwhile: [`BackdropLease::generation`] says which, when its answer has come.
    ///
    /// *Differs from the design's sketch* only in that `generation` is what the app expects to freeze, not something the
    /// protocol carries (`BackdropFreeze` names none).
    pub fn freeze(&self, generation: BackdropGeneration) -> BackdropLease {
        let _ = generation;
        let state = Rc::new(Cell::new(LeaseState::Pending));
        let mut guard = self.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let entry = inner
            .backdrops
            .get_mut(self.id.key())
            .expect("a live handle");
        entry.pending.push_back(Rc::clone(&state));
        let (wire, created_tag) = (entry.wire, entry.created_tag);
        // A failed connection is the next commit's to report, and no answer will ever come.
        let _ = inner.urgent(
            &Request::BackdropFreeze { id: wire },
            Urgent {
                after_tag: created_tag,
                stream: BACKDROP_STREAM | wire.0,
                replaceable: None,
            },
        );
        BackdropLease {
            state,
            id: self.id,
            scene: self.scene.clone(),
        }
    }
}

impl Drop for PreparedBackdrop {
    fn drop(&mut self) {
        self.scene.inner.borrow_mut().backdrop_release(self.id);
    }
}

/// A hold on one generation of a prepared set. Dropping it ends the hold.
pub struct BackdropLease {
    state: Rc<Cell<LeaseState>>,
    id: BackdropId,
    scene: Scene,
}

impl BackdropLease {
    /// The generation the compositor holds for this lease, once it has said (`Event::BackdropFrozen`); `None` before that and
    /// after the set is gone.
    pub fn generation(&self) -> Option<BackdropGeneration> {
        match self.state.get() {
            LeaseState::Held(g) => Some(g),
            LeaseState::Pending | LeaseState::Over => None,
        }
    }
}

impl Drop for BackdropLease {
    fn drop(&mut self) {
        if self.state.get() == LeaseState::Over {
            return;
        }
        self.state.set(LeaseState::Over);
        let mut guard = self.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let Some(entry) = inner.backdrops.get_mut(self.id.key()) else {
            return;
        };
        entry.held.retain(|l| !Rc::ptr_eq(l, &self.state));
        let (wire, created_tag) = (entry.wire, entry.created_tag);
        let _ = inner.urgent(
            &Request::BackdropRelease { id: wire },
            Urgent {
                after_tag: created_tag,
                stream: BACKDROP_STREAM | wire.0,
                replaceable: None,
            },
        );
    }
}

const _: () = assert!(MAX_BACKDROPS == 4);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::Group;
    use crate::scene::SceneOptions;
    use crate::scene::testing::{connect, drain};
    use compositor_protocol::{Event, Fill, GroupPose, Props, Rgba, Shape};

    fn shape() -> Shape {
        Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(1, 1, 1, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        }
    }

    fn spec(plates: Vec<GroupId>) -> BackdropSpec {
        BackdropSpec {
            wallpaper: None,
            plates,
            radius: 30.0,
            downsample: 4,
        }
    }

    #[test]
    fn the_set_is_deleted_before_the_groups_and_the_wallpaper_it_captured_whatever_order_they_are_dropped_in()
     {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let hero = scene.group(None, GroupPose::IDENTITY).unwrap();
        let dock = scene.group(None, GroupPose::IDENTITY).unwrap();
        let wallpaper = scene.shape(960, 480, 0, shape(), Props::at(0.0, 0.0));
        let backdrop = scene
            .prepare_backdrop(&BackdropSpec {
                wallpaper: Some(wallpaper.id()),
                ..spec(vec![hero.id(), dock.id()])
            })
            .unwrap();
        let (w, h, d, b) = (
            wallpaper.id(),
            hero.wire_id(),
            dock.wire_id(),
            backdrop.wire_id(),
        );
        scene.try_commit().unwrap();
        let first = drain(&mut comp);
        let Request::BackdropCreate { id, spec } = first
            .iter()
            .find(|r| matches!(r, Request::BackdropCreate { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(*id, b);
        assert_eq!(
            spec.plates,
            vec![PlateSource { group: h }, PlateSource { group: d }]
        );
        assert_eq!(
            (spec.wallpaper, spec.radius, spec.downsample),
            (Some(w), 30.0, 4)
        );
        // The inputs are dropped first; nothing of theirs is deleted while the set holds them.
        drop(wallpaper);
        drop(hero);
        drop(dock);
        assert!(!scene.has_pending(), "they wait for the set");
        drop(backdrop);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let at = |wanted: &Request| {
            sent.iter()
                .position(|r| r == wanted)
                .unwrap_or_else(|| panic!("{wanted:?} in {sent:?}"))
        };
        let set_deleted = at(&Request::BackdropDelete { id: b });
        for freed in [
            Request::GroupDelete { id: h },
            Request::GroupDelete { id: d },
            Request::Delete { id: w },
        ] {
            assert!(
                set_deleted < at(&freed),
                "captures cancelled before inputs freed: {freed:?}"
            );
        }
    }

    #[test]
    fn ceilings_and_bad_specs_are_errors_and_stage_nothing() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let groups: Vec<Group> = (0..9)
            .map(|_| scene.group(None, GroupPose::IDENTITY).unwrap())
            .collect();
        scene.try_commit().unwrap();
        let ids: Vec<GroupId> = groups.iter().map(Group::id).collect();
        assert!(matches!(
            scene.prepare_backdrop(&spec(ids.clone())),
            Err(UiError::Limit(Limit::Plates))
        ));
        let dup = [ids[0], ids[0]];
        assert!(matches!(
            scene.prepare_backdrop(&spec(dup.to_vec())),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.prepare_backdrop(&spec(vec![])),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.prepare_backdrop(&BackdropSpec {
                downsample: 3,
                ..spec(ids[..1].to_vec())
            }),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.prepare_backdrop(&BackdropSpec {
                radius: f32::NAN,
                ..spec(ids[..1].to_vec())
            }),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.prepare_backdrop(&BackdropSpec {
                radius: 65.0,
                ..spec(ids[..1].to_vec())
            }),
            Err(UiError::Invalid(_))
        ));
        assert!(!scene.has_pending());
        let sets: Vec<PreparedBackdrop> = (0..MAX_BACKDROPS)
            .map(|k| scene.prepare_backdrop(&spec(vec![ids[k]])).unwrap())
            .collect();
        assert!(matches!(
            scene.prepare_backdrop(&spec(vec![ids[5]])),
            Err(UiError::Limit(Limit::Backdrops))
        ));
        drop(sets);
        let stale = ids[8];
        drop(groups);
        assert!(matches!(
            scene.prepare_backdrop(&spec(vec![stale])),
            Err(UiError::Stale(Kind::Group))
        ));
    }

    #[test]
    fn refreshes_coalesce_and_ready_follows_the_compositors_events() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let hero = scene.group(None, GroupPose::IDENTITY).unwrap();
        let set = scene.prepare_backdrop(&spec(vec![hero.id()])).unwrap();
        assert_eq!(set.ready(), None);
        set.request_refresh(ContentGeneration(1));
        set.request_refresh(ContentGeneration(2));
        set.request_refresh(ContentGeneration(3));
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let refreshes: Vec<&Request> = sent
            .iter()
            .filter(|r| matches!(r, Request::BackdropRefresh { .. }))
            .collect();
        assert_eq!(
            refreshes,
            vec![&Request::BackdropRefresh {
                id: set.wire_id(),
                content: 3
            }]
        );
        scene.observe(&Event::BackdropReady {
            id: set.wire_id(),
            generation: 1,
            content: 3,
        });
        assert_eq!(
            (set.ready(), set.ready_content()),
            (Some(BackdropGeneration(1)), Some(ContentGeneration(3)))
        );
        // A late, older generation does not take the place of a newer one.
        scene.observe(&Event::BackdropReady {
            id: set.wire_id(),
            generation: 3,
            content: 9,
        });
        scene.observe(&Event::BackdropReady {
            id: set.wire_id(),
            generation: 2,
            content: 5,
        });
        assert_eq!(set.ready(), Some(BackdropGeneration(3)));
    }

    #[test]
    fn a_lease_is_asked_for_at_once_after_the_set_exists_learns_its_generation_and_releases_on_drop()
     {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let hero = scene.group(None, GroupPose::IDENTITY).unwrap();
        let set = scene.prepare_backdrop(&spec(vec![hero.id()])).unwrap();
        // The set is created by a commit that has not been made: the freeze waits for it, not the other way round.
        let lease = set.freeze(BackdropGeneration(1));
        assert!(drain(&mut comp).is_empty(), "nothing before the set exists");
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let create = sent
            .iter()
            .position(|r| matches!(r, Request::BackdropCreate { .. }))
            .unwrap();
        let freeze = sent
            .iter()
            .position(|r| matches!(r, Request::BackdropFreeze { .. }))
            .unwrap();
        assert!(create < freeze);
        assert_eq!(lease.generation(), None, "no answer yet");
        scene.observe(&Event::BackdropFrozen {
            id: set.wire_id(),
            generation: 4,
        });
        assert_eq!(lease.generation(), Some(BackdropGeneration(4)));
        // Later leases are answered in order; one dropped before its answer still consumes it.
        let second = set.freeze(BackdropGeneration(4));
        let third = set.freeze(BackdropGeneration(4));
        drop(second);
        scene.observe(&Event::BackdropFrozen {
            id: set.wire_id(),
            generation: 4,
        });
        scene.observe(&Event::BackdropFrozen {
            id: set.wire_id(),
            generation: 5,
        });
        assert_eq!(
            third.generation(),
            Some(BackdropGeneration(5)),
            "answers keep their order"
        );
        drop(lease);
        drop(third);
        let sent = drain(&mut comp);
        let releases = sent
            .iter()
            .filter(|r| matches!(r, Request::BackdropRelease { .. }))
            .count();
        let freezes = sent
            .iter()
            .filter(|r| matches!(r, Request::BackdropFreeze { .. }))
            .count();
        assert_eq!((freezes, releases), (2, 3), "every lease ends: {sent:?}");
        // Deleting the set ends the leases it still has: nothing is released afterwards.
        let held = set.freeze(BackdropGeneration(5));
        drop(set);
        assert_eq!(held.generation(), None);
        drop(held);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            sent.iter()
                .any(|r| matches!(r, Request::BackdropDelete { .. }))
        );
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::BackdropRelease { .. })),
            "{sent:?}"
        );
    }
}
