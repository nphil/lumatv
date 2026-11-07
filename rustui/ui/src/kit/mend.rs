//! A part made once and then kept (the profile chip, a focus plate) is made again when the compositor refused any of it, since
//! nothing else would ever make it again. A part refused once more waits longer each time, so that a scene at its ceiling is not
//! asked the same thing on every frame.

use ui::scene::{Scene, Watch};

/// The wait before the second try. It doubles with each try after it, at most [`MOST_DOUBLINGS`] times (to 32 s).
const FIRST_WAIT_MS: u64 = 1_000;
const MOST_DOUBLINGS: u32 = 5;

/// What became of the latest making of a kept part.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mend {
    /// The latest making, until it came through whole.
    watch: Option<Watch>,
    /// Makings again in a row that were refused too.
    tries: u32,
    /// The time the part may be made again (ms).
    not_before: u64,
}

impl Mend {
    /// The part is about to be made: what is built from now on is held to what the compositor makes of it.
    pub fn watch(&mut self, scene: &Scene) {
        self.watch = Some(scene.watch());
    }

    /// Whether to make the part again now: something was refused since its latest making and the wait after the latest try is over.
    /// A making that came through whole ends the watch and the waits.
    pub fn due(&mut self, scene: &Scene, now_ms: u64) -> bool {
        let Some(watch) = self.watch else {
            return false;
        };
        match scene.came_through(watch) {
            None => false,
            Some(true) => {
                *self = Self::default();
                false
            }
            Some(false) if now_ms < self.not_before => false,
            Some(false) => {
                self.not_before = now_ms + (FIRST_WAIT_MS << self.tries.min(MOST_DOUBLINGS));
                self.tries += 1;
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::Event;

    #[test]
    fn a_refused_part_is_due_at_once_then_after_doubling_waits_and_never_once_it_came_through() {
        let h = ui::testing::harness();
        let scene = &h.ui.scene;
        let refuse = || scene.observe(&Event::Error { message: "more than 256 layers".into() });
        let mut mend = Mend::default();
        assert!(!mend.due(scene, 0), "nothing made, nothing to mend");
        mend.watch(scene);
        refuse();
        assert!(mend.due(scene, 10), "the first try is at once");
        mend.watch(scene);
        refuse();
        assert!(!mend.due(scene, 1_009));
        assert!(mend.due(scene, 1_010), "a second later");
        mend.watch(scene);
        refuse();
        assert!(!mend.due(scene, 3_009));
        assert!(mend.due(scene, 3_010), "two seconds after that");
        let mut now = 3_010;
        for _ in 0..8 {
            now += 32_000;
            mend.watch(scene);
            refuse();
            assert!(mend.due(scene, now));
        }
        assert_eq!(mend.not_before - now, 32_000, "the wait stops growing");
        // Made again and shown with nothing refused: done, and a later refusal is no business of the part's.
        mend.watch(scene);
        let tag = scene.try_commit().expect("a commit").get();
        scene.shown(tag);
        assert!(!mend.due(scene, now));
        refuse();
        assert!(!mend.due(scene, now));
    }
}
