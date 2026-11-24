//! The page backdrops of the games Home and the Library show, decoded ahead on a thread of their own: a game page that opens finds its
//! art held (decoding it takes about 450 ms on the set, and OK may follow a move within milliseconds).
//!
//! At most [`MAX`] pictures are held, 1920 x 1080 x 4 bytes (8.3 MB) each, so 33 MB. The pictures asked for are ranked by how near their
//! tile is to focus; a picture that lands beyond the limit drops the one furthest down the ranking.
use super::{Key, Loader};
use lumagame_meta::Image;
use std::{collections::BTreeSet, io};

/// The most backdrops held: the focused game's, its neighbours' in the row or grid.
pub const MAX: usize = 4;
/// Niceness of the decode thread: the frame loop and the decodes the page waits on come first.
const NICE: i32 = 10;

pub struct Ahead {
    loader: Loader,
    /// The pictures asked for, nearest focus first, at most [`MAX`].
    wanted: Vec<Key>,
    held: Vec<(Key, Image)>,
    failed: BTreeSet<Key>,
    flying: Option<Key>,
    /// Whether Home or the Library was up at the last step: the pictures are decoded only for those.
    browsing: bool,
}
impl Ahead {
    pub fn new() -> io::Result<Self> {
        Ok(Self { loader: Loader::with_threads(1, NICE)?, wanted: Vec::new(), held: Vec::new(), failed: BTreeSet::new(), flying: None, browsing: false })
    }
    /// Names the pictures to hold, each with how far its tile is from focus. Pictures held that are no longer named stay until the room
    /// is wanted; none are named by a page that has no such tiles, and then none are held.
    pub fn want(&mut self, mut keys: Vec<(u32, Key)>) {
        if keys.is_empty() {
            self.held.clear();
            self.failed.clear();
        }
        keys.sort_by_key(|(near, _)| *near);
        let mut wanted: Vec<Key> = Vec::with_capacity(MAX);
        for (_, key) in keys {
            if wanted.len() < MAX && !wanted.contains(&key) {
                wanted.push(key);
            }
        }
        self.wanted = wanted;
    }
    pub fn poll(&mut self) {
        while let Some(done) = self.loader.poll() {
            if self.flying.as_ref() == Some(&done.key) {
                self.flying = None;
            }
            match done.image {
                Ok(image) if !self.wanted.is_empty() => self.keep(done.key, image),
                Ok(_) => {}
                Err(_) => {
                    self.failed.insert(done.key);
                }
            }
        }
    }
    pub(super) fn keep(&mut self, key: Key, image: Image) {
        self.held.retain(|(held, _)| *held != key);
        self.held.push((key, image));
        while self.held.len() > MAX {
            let furthest = self.held.iter().enumerate().max_by_key(|(_, (key, _))| self.rank(key)).map(|(i, _)| i);
            if let Some(i) = furthest {
                self.held.swap_remove(i);
            }
        }
    }
    /// Where the picture stands in the ranking: nearest first, and last when it is not named.
    fn rank(&self, key: &Key) -> usize {
        self.wanted.iter().position(|wanted| wanted == key).unwrap_or(usize::MAX)
    }
    /// The nearest picture named that is neither held, nor failed, nor on its way.
    fn next(&self) -> Option<&Key> {
        self.wanted.iter().find(|key| {
            self.flying.as_ref() != Some(key) && !self.failed.contains(key) && !self.held.iter().any(|(held, _)| held.same_picture(key))
        })
    }
    /// Starts the next decode when none is under way, Home or the Library is up and no list is scrolling.
    pub fn run(&mut self, browsing: bool, scrolling: bool) {
        self.browsing = browsing;
        if !browsing || scrolling || self.flying.is_some() {
            return;
        }
        let Some(key) = self.next().cloned() else { return };
        if self.loader.request(key.clone(), false) {
            self.flying = Some(key);
        }
    }
    /// Whether there is work under way, or waiting for the lists to settle.
    pub fn busy(&self) -> bool {
        self.loader.busy() || (self.browsing && self.next().is_some())
    }
    /// Whether the picture is on its way: a decode of the page waits for it rather than begin its own.
    pub fn flying(&self, key: &Key) -> bool {
        self.flying.as_ref().is_some_and(|flying| flying.same_picture(key))
    }
    pub fn holds(&self, key: &Key) -> bool {
        self.held.iter().any(|(held, _)| held.same_picture(key))
    }
    /// The picture, which the page now holds in its stead.
    pub fn take(&mut self, key: &Key) -> Option<(Key, Image)> {
        let at = self.held.iter().position(|(held, _)| held.same_picture(key))?;
        Some(self.held.swap_remove(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::art::{Spec, Stage};

    fn key(name: &str) -> Key {
        let stage = Stage { art_height: 1, fade_from: 0, fade_to: 1, canvas: [0; 3], scrim: [0; 4], dim: false, curtain: false };
        Key { path: name.into(), spec: Spec { width: 1, height: 1, radius: 0, logo: false, stage: Some(stage) } }
    }
    fn land(ahead: &mut Ahead, name: &str) {
        ahead.keep(key(name), Image { width: 1, height: 1, rgba: vec![0; 4] });
    }

    #[test]
    fn the_nearest_picture_not_yet_held_is_decoded_first_and_none_while_scrolling() {
        let mut ahead = Ahead::new().unwrap();
        ahead.want(vec![(2, key("c")), (0, key("a")), (1, key("b")), (1, key("b"))]);
        assert_eq!(ahead.wanted, [key("a"), key("b"), key("c")]);
        ahead.run(true, true);
        assert!(ahead.flying.is_none(), "the lists are scrolling");
        ahead.run(true, false);
        assert_eq!(ahead.flying, Some(key("a")));
        land(&mut ahead, "a");
        ahead.flying = None;
        assert_eq!(ahead.next(), Some(&key("b")));
    }

    #[test]
    fn no_more_than_the_limit_is_held_and_the_furthest_goes_first() {
        let mut ahead = Ahead::new().unwrap();
        ahead.want((0..6).map(|n| (n, key(&n.to_string()))).collect());
        assert_eq!(ahead.wanted.len(), MAX);
        for name in ["5", "0", "1", "2", "3"] {
            land(&mut ahead, name);
        }
        let mut names: Vec<_> = ahead.held.iter().map(|(key, _)| key.path.to_str().unwrap().to_owned()).collect();
        names.sort();
        assert_eq!(names, ["0", "1", "2", "3"]);
    }

    #[test]
    fn a_page_with_no_tiles_holds_nothing() {
        let mut ahead = Ahead::new().unwrap();
        land(&mut ahead, "a");
        ahead.want(Vec::new());
        assert!(!ahead.holds(&key("a")));
        assert!(!ahead.busy());
    }
}
