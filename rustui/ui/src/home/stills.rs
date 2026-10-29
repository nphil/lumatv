//! Still pictures for Home's cameras card, through the same helper frame route as CameraFrame.qml.
//! The Cameras planner bounds concurrent asks and paces each next ask after its answer. Home uses a
//! separate tag band so its answers cannot reach the full-screen viewer's picture store.
use crate::cameras::pics::{Planner, Want};
use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
use std::collections::HashMap;

const FIRST: u64 = 0x0CB0_0000;
const LAST: u64 = 0x0CBF_FFFF;

pub struct Stills {
    planner: Planner,
    wants: Vec<Want>,
    configured: Vec<(String, u32, u32, bool)>,
    pictures: HashMap<String, Picture>,
    pub revision: u64,
}

impl Default for Stills {
    fn default() -> Self {
        Self { planner: Planner::with_tags(FIRST, LAST), wants: Vec::new(), configured: Vec::new(), pictures: HashMap::new(), revision: 0 }
    }
}

impl Stills {
    /// Only the rows actually drawn need pictures. Offline rows retain their last good still.
    pub fn configure(&mut self, rows: &[(String, u32, u32, bool)]) {
        if self.configured == rows { return; }
        self.configured.clear();
        self.configured.extend_from_slice(rows);
        self.wants.clear();
        for (id, w, h, online) in rows {
            if *online && !self.wants.iter().any(|v| v.key == *id) {
                self.wants.push(Want::repeating(id, &format!("/v1/cameras/{}/frame?w={w}&h={h}", crate::cameras::sheet::encode(id)), 1000));
            }
        }
        self.pictures.retain(|id, _| rows.iter().any(|r| r.0 == *id));
    }

    pub fn picture(&self, id: &str) -> Option<&Picture> {
        self.pictures.get(id)
    }

    pub fn update(&mut self, input: &Input, visible: bool, now: u64) -> Vec<Effect> {
        if !visible {
            self.planner.quiesce();
            return Vec::new();
        }
        let mut out = Vec::new();
        match input {
            Input::Bytes { tag, status, bytes } if (FIRST..=LAST).contains(tag) => {
                if let Some(key) = self.planner.key_of(*tag).map(str::to_owned) {
                    let picture = (*status == 200).then(|| picture::decode(bytes).ok()).flatten();
                    let ok = picture.is_some();
                    if let Some(picture) = picture {
                        if self.wants.iter().any(|w| w.key == key) {
                            self.pictures.insert(key, picture);
                            self.revision += 1;
                            out.push(Effect::Redraw);
                        }
                    }
                    self.planner.arrived(*tag, *status, ok, now);
                }
            }
            Input::Fetched { tag, status, .. } if (FIRST..=LAST).contains(tag) => {
                self.planner.arrived(*tag, *status, false, now);
            }
            _ => {}
        }
        out.extend(self.planner.plan(&self.wants, now, 2));
        out
    }

    pub fn due_at(&self, visible: bool) -> Option<u64> {
        if !visible || self.wants.is_empty() { None } else { self.planner.due_at(&self.wants).or((self.planner.in_flight() == 0).then_some(0)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stills_are_small_paced_bounded_and_quiet_when_home_is_not_visible() {
        let mut s = Stills::default();
        s.configure(&[("front/door".into(), 100, 56, true), ("back".into(), 100, 56, false)]);
        assert_eq!(s.due_at(true), Some(0));
        let effects = s.update(&Input::Tick, true, 1000);
        assert_eq!(effects.len(), 1);
        let Effect::Fetch { tag, path } = &effects[0] else { panic!("frame fetch") };
        assert!((FIRST..=LAST).contains(tag));
        assert_eq!(path, "/v1/cameras/front%2Fdoor/frame?w=100&h=56");
        assert!(s.update(&Input::Tick, true, 1100).is_empty());
        // An undecodable still is not a picture; the helper is not hammered after a refusal.
        s.update(&Input::Bytes { tag: *tag, status: 200, bytes: b"bad".to_vec() }, true, 1200);
        assert_eq!(s.due_at(true), Some(2200));
        assert!(s.update(&Input::Tick, true, 2199).is_empty());
        assert_eq!(s.update(&Input::Tick, true, 2200).len(), 1);
        assert!(s.update(&Input::Tick, false, 2300).is_empty());
        assert_eq!(s.due_at(false), None);
        assert!(s.picture("front/door").is_none());
    }

    #[test]
    fn a_decoded_still_is_kept_after_a_refusal_and_late_answers_are_ignored_after_home_hides() {
        let mut s = Stills::default();
        s.configure(&[("front".into(), 100, 56, true)]);
        let effects = s.update(&Input::Tick, true, 1000);
        let Effect::Fetch { tag, .. } = effects[0] else { panic!("a fetch") };
        let bytes = include_bytes!("../../../../assets/icon160.png").to_vec();
        assert!(s.update(&Input::Bytes { tag, status: 200, bytes: bytes.clone() }, true, 1100).contains(&Effect::Redraw));
        assert!(s.picture("front").is_some());
        assert_eq!(s.revision, 1);
        assert_eq!(s.due_at(true), Some(2100));
        let effects = s.update(&Input::Tick, true, 2100);
        let Effect::Fetch { tag, .. } = effects[0] else { panic!("a fetch") };
        s.update(&Input::Fetched { tag, status: 503, data: serde_json::Value::Null }, true, 2200);
        assert!(s.picture("front").is_some());
        let effects = s.update(&Input::Tick, true, 3200);
        let Effect::Fetch { tag, .. } = effects[0] else { panic!("a fetch") };
        s.update(&Input::Tick, false, 3250);
        s.update(&Input::Bytes { tag, status: 200, bytes }, true, 3300);
        assert_eq!(s.revision, 1);
        s.configure(&[]);
        assert!(s.picture("front").is_none());
        assert_eq!(s.due_at(true), None);
    }

    #[test]
    fn at_most_two_frames_are_outstanding_even_with_more_rows() {
        let mut s = Stills::default();
        s.configure(&[("one".into(), 100, 56, true), ("two".into(), 100, 56, true), ("three".into(), 100, 56, true)]);
        let effects = s.update(&Input::Tick, true, 1000);
        assert_eq!(effects.len(), 2);
        assert!(s.update(&Input::Tick, true, 1100).is_empty());
        let Effect::Fetch { tag, .. } = effects[0] else { panic!("a fetch") };
        let effects = s.update(&Input::Fetched { tag, status: 503, data: serde_json::Value::Null }, true, 1200);
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::Fetch { path, .. } if path.contains("/three/frame")));
    }
}
