//! What the stage art's layer shows, and when it changes: the pictures held, and the dip from one to the next.
//!
//! A dip is: focus rests on the new picture for `ART_REST_MS`, the layer fades out over `ART_DIP_MS`, the new picture goes into it,
//! and it fades in over `ART_DIP_MS`. Nothing changes while a page scrolls, and with reduced motion the picture is cut at the end of
//! the rest. A layer that opens or closes over the same art skips the rest, as only the copy shown differs. This holds no layer and
//! no clock: [`Dip::step`] is given the time and answers with what the layer is to do.
use super::{Done, Key};
use crate::game::screens::parts::motion::{ART_DIP_MS, ART_REST_MS};
use lumagame_meta::Image;
use std::collections::BTreeSet;

/// The decoded pictures held. A decode is asked for only when none is in the air, and then only the shown picture stays, so no more
/// than two stage-size pictures (the one on the layer, the one to come) and one copy under the scrim are ever resident.
#[derive(Default)]
pub struct Pictures {
    held: Vec<Held>,
    failed: BTreeSet<Key>,
    flying: Option<(Key, bool)>,
}
struct Held {
    key: Key,
    image: Image,
    dim: Option<Image>,
}
impl Pictures {
    /// Whether the key's picture, in the copy it shows, can go on the layer now.
    pub fn has(&self, key: &Key) -> bool {
        self.held.iter().any(|held| held.key.same_picture(key) && (!key.dim() || held.dim.is_some()))
    }
    pub fn image(&self, key: &Key) -> Option<&Image> {
        let held = self.held.iter().find(|held| held.key.same_picture(key))?;
        if key.dim() { held.dim.as_ref() } else { Some(&held.image) }
    }
    pub fn failed(&self, key: &Key) -> bool {
        self.failed.iter().any(|failed| failed.same_picture(key))
    }
    /// Whether the key's picture is yet to be asked for: not held, not failed, not on its way.
    pub fn needs(&self, key: &Key) -> bool {
        !self.has(key)
            && !self.failed(key)
            && !self.flying.as_ref().is_some_and(|(flying, dim)| flying.same_picture(key) && (*dim || !key.dim()))
    }
    /// Clears the way for a decode: everything but the shown picture goes. False while a decode is in the air, which lands in the room
    /// it was given.
    pub fn make_room(&mut self, shown: Option<&Key>) -> bool {
        if self.flying.is_some() {
            return false;
        }
        self.held.retain(|held| shown.is_some_and(|shown| held.key.same_picture(shown)));
        true
    }
    /// A picture decoded ahead takes its place, without a copy under the scrim.
    pub fn adopt(&mut self, key: Key, image: Image) {
        self.held.retain(|held| held.key != key);
        self.held.push(Held { key, image, dim: None });
    }
    /// Hands out the held picture that has no copy under the scrim, to have it made: the picture is on its way until it lands again.
    pub fn lend(&mut self, key: &Key) -> Option<(Key, Image)> {
        if self.flying.is_some() {
            return None;
        }
        let at = self.held.iter().position(|held| held.key.same_picture(key) && held.dim.is_none())?;
        let held = self.held.swap_remove(at);
        self.flying = Some((held.key.clone(), true));
        Some((held.key, held.image))
    }
    /// What was lent is not coming back.
    pub fn lost(&mut self) {
        self.flying = None;
    }
    pub fn fly(&mut self, key: Key, dim: bool) {
        self.flying = Some((key, dim));
    }
    pub fn arrive(&mut self, done: Done) {
        self.flying = None;
        match done.image {
            Ok(image) => {
                // Only the newest picture keeps its copy under the scrim: it is the one that goes on the layer next.
                if done.dim.is_some() {
                    for held in &mut self.held {
                        held.dim = None;
                    }
                }
                self.held.retain(|held| held.key != done.key);
                self.held.push(Held { key: done.key, image, dim: done.dim });
            }
            Err(_) => {
                self.failed.insert(done.key);
            }
        }
    }
}

/// What the layer is told to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Cue {
    /// The page's first picture goes up.
    Place(Key),
    /// The picture on the layer fades out.
    Out,
    /// The picture goes into the layer, and fades in from dark when `fade`.
    Swap { key: Key, fade: bool },
    /// The dip was called off: the picture on the layer fades back in.
    In,
    /// The picture cannot be had: the page has no art.
    Drop,
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// A picture to decode, with its copy under the scrim.
    pub request: Option<Key>,
    pub cue: Option<Cue>,
}

#[derive(Default)]
pub struct Dip {
    /// What is on the layer: the picture that went in last.
    shown: Option<Key>,
    wanted: Option<Want>,
    phase: Phase,
}
struct Want {
    key: Key,
    /// The clock when the page began to want it: the rest runs from here.
    since: u64,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// Fading out; at `until` the new picture goes in, or is waited for.
    Out { until: u64 },
    /// Fading in; at `until` the dip is over.
    In { until: u64 },
}
impl Dip {
    /// The page's plan names the picture it wants on the stage.
    pub fn want(&mut self, key: Key, now: u64) {
        match &mut self.wanted {
            Some(want) if want.key == key => {}
            // The same picture in the other copy: a layer opened or closed, and the rest that was running goes on.
            Some(want) if want.key.same_picture(&key) => want.key = key,
            wanted => *wanted = Some(Want { key, since: now }),
        }
    }
    pub fn wanted(&self) -> Option<&Key> {
        self.wanted.as_ref().map(|want| &want.key)
    }
    pub fn shown(&self) -> Option<&Key> {
        self.shown.as_ref()
    }
    /// Whether the dip has something left to do: the wanted picture is not the one shown, or a fade is running.
    pub fn pending(&self) -> bool {
        self.shown.as_ref().is_some_and(|shown| self.phase != Phase::Idle || self.wanted.as_ref().is_some_and(|want| want.key != *shown))
    }
    /// Whether the layer is faded out and waiting for its picture: it draws nothing.
    #[cfg(test)]
    pub fn dark(&self, now: u64) -> bool {
        matches!(self.phase, Phase::Out { until } if now >= until)
    }
    pub fn step(&mut self, now: u64, scrolling: bool, reduced: bool, pictures: &Pictures) -> Step {
        match self.phase {
            Phase::In { until } if now < until => return Step::default(),
            Phase::In { .. } => self.phase = Phase::Idle,
            Phase::Out { until } if now < until => return Step::default(),
            Phase::Out { .. } => return self.out_ended(now, scrolling, pictures),
            Phase::Idle => {}
        }
        self.idle(now, scrolling, reduced, pictures)
    }
    /// Whether the wanted picture may replace the shown one: focus has rested on it, or it is the same picture in the other copy.
    fn due(&self, now: u64, scrolling: bool) -> bool {
        let (Some(shown), Some(want)) = (&self.shown, &self.wanted) else { return false };
        !scrolling && (shown.same_picture(&want.key) || now >= want.since + u64::from(ART_REST_MS))
    }
    fn idle(&mut self, now: u64, scrolling: bool, reduced: bool, pictures: &Pictures) -> Step {
        let mut step = Step::default();
        let Some(want) = &self.wanted else { return step };
        let key = &want.key;
        let Some(shown) = &self.shown else {
            // The page's first picture goes up when it is decoded.
            if pictures.has(key) {
                step.cue = Some(Cue::Place(key.clone()));
                self.shown = Some(key.clone());
            } else if pictures.needs(key) {
                step.request = Some(key.clone());
            }
            return step;
        };
        if shown == key || !self.due(now, scrolling) {
            return step;
        }
        if pictures.failed(key) {
            self.shown = None;
            step.cue = Some(Cue::Drop);
            return step;
        }
        if pictures.needs(key) {
            step.request = Some(key.clone());
        }
        if reduced {
            if pictures.has(key) {
                step.cue = Some(Cue::Swap { key: key.clone(), fade: false });
                self.shown = Some(key.clone());
            }
        } else {
            self.phase = Phase::Out { until: now + u64::from(ART_DIP_MS) };
            step.cue = Some(Cue::Out);
        }
        step
    }
    /// The layer is dark. The picture wanted now goes in, or is waited for; if a scroll or a newer want has taken it away, the dip is
    /// called off and the picture on the layer comes back.
    fn out_ended(&mut self, now: u64, scrolling: bool, pictures: &Pictures) -> Step {
        let mut step = Step::default();
        let Some(want) = &self.wanted else { return self.call_off(now) };
        let key = &want.key;
        if self.shown.as_ref() == Some(key) || !self.due(now, scrolling) {
            return self.call_off(now);
        }
        if pictures.failed(key) {
            self.shown = None;
            self.phase = Phase::Idle;
            step.cue = Some(Cue::Drop);
        } else if pictures.has(key) {
            step.cue = Some(Cue::Swap { key: key.clone(), fade: true });
            self.shown = Some(key.clone());
            self.phase = Phase::In { until: now + u64::from(ART_DIP_MS) };
        } else if pictures.needs(key) {
            step.request = Some(key.clone());
        }
        step
    }
    fn call_off(&mut self, now: u64) -> Step {
        self.phase = Phase::In { until: now + u64::from(ART_DIP_MS) };
        Step { request: None, cue: Some(Cue::In) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::art::{Spec, Stage};

    fn key(name: &str, dim: bool) -> Key {
        let stage = Stage { art_height: 8, fade_from: 0, fade_to: 8, canvas: [0; 3], scrim: [0; 4], dim, curtain: false };
        Key { path: name.into(), spec: Spec { width: 8, height: 8, radius: 0, logo: false, stage: Some(stage) } }
    }
    fn image() -> Image {
        Image { width: 1, height: 1, rgba: vec![0; 4] }
    }
    fn land(pictures: &mut Pictures, name: &str, dim: bool) {
        pictures.arrive(Done { key: key(name, false), image: Ok(image()), dim: dim.then(image) });
    }
    /// A dip showing "a" with "b" wanted since 1000, on a clock that is whatever each call says.
    fn a_to_b(pictures: &mut Pictures) -> Dip {
        land(pictures, "a", false);
        land(pictures, "b", false);
        let mut dip = Dip::default();
        dip.want(key("a", false), 0);
        assert_eq!(dip.step(0, false, false, pictures).cue, Some(Cue::Place(key("a", false))));
        dip.want(key("b", false), 1_000);
        dip
    }
    fn cue(dip: &mut Dip, now: u64, scrolling: bool, reduced: bool, pictures: &Pictures) -> Option<Cue> {
        dip.step(now, scrolling, reduced, pictures).cue
    }

    #[test]
    fn art_rests_then_dips_out_swaps_and_fades_in() {
        let mut pictures = Pictures::default();
        let mut dip = a_to_b(&mut pictures);
        assert_eq!(cue(&mut dip, 1_249, false, false, &pictures), None);
        assert_eq!(cue(&mut dip, 1_250, false, false, &pictures), Some(Cue::Out));
        assert_eq!(cue(&mut dip, 1_349, false, false, &pictures), None);
        assert_eq!(cue(&mut dip, 1_350, false, false, &pictures), Some(Cue::Swap { key: key("b", false), fade: true }));
        assert!(dip.pending());
        assert_eq!(cue(&mut dip, 1_449, false, false, &pictures), None);
        assert!(dip.pending());
        assert_eq!(cue(&mut dip, 1_450, false, false, &pictures), None);
        assert!(!dip.pending());
    }

    #[test]
    fn a_scroll_holds_the_picture_and_calls_off_a_dip_under_way() {
        let mut pictures = Pictures::default();
        let mut dip = a_to_b(&mut pictures);
        assert_eq!(cue(&mut dip, 2_000, true, false, &pictures), None);
        assert_eq!(cue(&mut dip, 2_100, false, false, &pictures), Some(Cue::Out));
        assert_eq!(cue(&mut dip, 2_200, true, false, &pictures), Some(Cue::In));
        assert_eq!(dip.shown(), Some(&key("a", false)));
        assert_eq!(cue(&mut dip, 2_300, false, false, &pictures), Some(Cue::Out));
    }

    #[test]
    fn a_newer_want_during_the_fade_out_restarts_the_rest() {
        let mut pictures = Pictures::default();
        let mut dip = a_to_b(&mut pictures);
        land(&mut pictures, "c", false);
        assert_eq!(cue(&mut dip, 1_250, false, false, &pictures), Some(Cue::Out));
        dip.want(key("c", false), 1_300);
        assert_eq!(cue(&mut dip, 1_350, false, false, &pictures), Some(Cue::In));
        assert_eq!(cue(&mut dip, 1_450, false, false, &pictures), None);
        assert_eq!(cue(&mut dip, 1_550, false, false, &pictures), Some(Cue::Out));
    }

    #[test]
    fn reduced_motion_cuts_after_the_rest() {
        let mut pictures = Pictures::default();
        let mut dip = a_to_b(&mut pictures);
        assert_eq!(cue(&mut dip, 1_249, false, true, &pictures), None);
        assert_eq!(cue(&mut dip, 1_250, false, true, &pictures), Some(Cue::Swap { key: key("b", false), fade: false }));
        assert!(!dip.pending());
    }

    #[test]
    fn a_layer_over_the_same_art_dips_to_its_copy_without_a_rest() {
        let mut pictures = Pictures::default();
        land(&mut pictures, "a", true);
        let mut dip = Dip::default();
        dip.want(key("a", false), 0);
        assert_eq!(cue(&mut dip, 0, false, false, &pictures), Some(Cue::Place(key("a", false))));
        dip.want(key("a", true), 5_000);
        assert_eq!(cue(&mut dip, 5_000, false, false, &pictures), Some(Cue::Out));
        assert_eq!(cue(&mut dip, 5_100, false, false, &pictures), Some(Cue::Swap { key: key("a", true), fade: true }));
    }

    #[test]
    fn the_layer_stays_dark_while_the_picture_decodes() {
        let mut pictures = Pictures::default();
        land(&mut pictures, "a", false);
        let mut dip = Dip::default();
        dip.want(key("a", false), 0);
        assert_eq!(cue(&mut dip, 0, false, false, &pictures), Some(Cue::Place(key("a", false))));
        dip.want(key("b", false), 1_000);
        assert_eq!(dip.step(1_250, false, false, &pictures), Step { request: Some(key("b", false)), cue: Some(Cue::Out) });
        assert_eq!(dip.step(1_350, false, false, &pictures), Step { request: Some(key("b", false)), cue: None });
        assert!(dip.dark(1_350));
        land(&mut pictures, "b", false);
        assert_eq!(cue(&mut dip, 1_400, false, false, &pictures), Some(Cue::Swap { key: key("b", false), fade: true }));
        assert!(!dip.dark(1_400));
    }

    #[test]
    fn art_that_will_not_decode_leaves_the_page_without_it() {
        let mut pictures = Pictures::default();
        let mut dip = a_to_b(&mut pictures);
        dip.want(key("c", false), 1_100);
        pictures.arrive(Done { key: key("c", false), image: Err("unreadable".into()), dim: None });
        assert_eq!(cue(&mut dip, 1_350, false, false, &pictures), Some(Cue::Drop));
        assert!(!dip.pending());
        assert_eq!(dip.step(1_400, false, false, &pictures), Step::default());
    }

    #[test]
    fn two_pictures_and_one_copy_under_the_scrim_are_held_at_most() {
        let mut pictures = Pictures::default();
        let (a, b) = (key("a", false), key("b", false));
        assert!(pictures.make_room(None));
        pictures.fly(a.clone(), true);
        assert!(!pictures.make_room(None));
        assert!(!pictures.needs(&a));
        land(&mut pictures, "a", true);
        assert!(pictures.make_room(Some(&a)));
        pictures.fly(b.clone(), true);
        land(&mut pictures, "b", true);
        assert_eq!(pictures.held.len(), 2);
        assert_eq!(pictures.held.iter().filter(|held| held.dim.is_some()).count(), 1);
        assert!(!pictures.has(&key("a", true)) && pictures.has(&key("b", true)));
        assert!(pictures.make_room(Some(&b)));
        assert_eq!(pictures.held.len(), 1);
    }
}
