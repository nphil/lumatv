//! The pictures the screens ask the helper for, and when: a tile's still, the full view's, the cursor's thumbnail, the cards' pictures
//! (`docs/research/cameras-ui-spec.md`, "Picture pacing"). A picture asked for again and again is paced by *the next ask after the last
//! arrived plus the interval*, so a slow helper is never asked faster than it answers; a picture asked once is asked a few times at
//! most. One ask a key is in flight at a time, and what the screen no longer wants is asked for no more. The decoded pictures are the
//! host's ([`super::store`]); this only says what to fetch and what the answers mean.

use super::grid::Pic;
use crate::kit::screen::Effect;

/// The tags of the asks the cameras make (the settings kind's are `0x5E00_0000` and up).
pub const TAG_FIRST: u64 = 0x0CA0_0000;
pub const TAG_LAST: u64 = 0x0CAF_FFFF;

pub fn owns(tag: u64) -> bool {
    (TAG_FIRST..=TAG_LAST).contains(&tag)
}

/// A picture the view wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    /// The key it is kept and drawn under.
    pub key: String,
    pub path: String,
    /// The pause between one arrival and the next ask; 0 asks once (and again, a few times, only after a refusal).
    pub every_ms: u64,
}

impl Want {
    pub fn repeating(key: &str, path: &str, every_ms: u64) -> Self {
        Self { key: key.to_string(), path: path.to_string(), every_ms }
    }

    pub fn once(key: &str, path: &str) -> Self {
        Self { key: key.to_string(), path: path.to_string(), every_ms: 0 }
    }
}

/// How many times a picture asked for once is asked for after refusals before it is given up (the card stays a glyph on a quiet tile).
pub const TRIES: u32 = 3;
/// A refused ask of a repeating picture is asked again after at least this long.
pub const RETRY_MS: u64 = 500;
/// After a refusal that says the helper is busy.
pub const BUSY_MS: u64 = 1000;

#[derive(Clone, Debug)]
struct Slot {
    key: String,
    path: String,
    every_ms: u64,
    in_flight: Option<u64>,
    next_at: u64,
    pic: Pic,
    tries: u32,
    done: bool,
}

/// What an answer was about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub key: String,
    /// A picture arrived (and decoded); false for a refusal.
    pub ok: bool,
}

#[derive(Debug)]
pub struct Planner {
    slots: Vec<Slot>,
    counter: u64,
}

impl Default for Planner {
    fn default() -> Self {
        Self::new()
    }
}

impl Planner {
    pub fn new() -> Self {
        Self { slots: Vec::new(), counter: TAG_FIRST }
    }

    fn slot_mut(&mut self, key: &str) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.key == key)
    }

    /// A tag for an ask of the app's own (a request of the helper, not a picture).
    pub fn next_tag(&mut self) -> u64 {
        let t = self.counter;
        self.counter = if t >= TAG_LAST { TAG_FIRST } else { t + 1 };
        t
    }

    /// What to ask for now: each wanted picture that is due and not in flight, at most `cap` in flight in all.
    pub fn plan(&mut self, wants: &[Want], now: u64, cap: usize) -> Vec<Effect> {
        let mut out = Vec::new();
        for w in wants {
            match self.slot_mut(&w.key) {
                Some(s) if s.path != w.path => {
                    // a new moment for the same place (the bubble followed the cursor): ask afresh, the old answer is not it
                    s.path = w.path.clone();
                    s.every_ms = w.every_ms;
                    s.in_flight = None;
                    s.done = false;
                    s.tries = 0;
                    s.next_at = now;
                }
                Some(s) => s.every_ms = w.every_ms,
                None => self.slots.push(Slot { key: w.key.clone(), path: w.path.clone(), every_ms: w.every_ms, in_flight: None, next_at: now, pic: Pic::default(), tries: 0, done: false }),
            }
        }
        let mut flying = self.slots.iter().filter(|s| s.in_flight.is_some()).count();
        for w in wants {
            if flying >= cap {
                break;
            }
            let due = self.slots.iter().any(|s| s.key == w.key && !s.done && s.in_flight.is_none() && now >= s.next_at);
            if !due {
                continue;
            }
            let tag = self.next_tag();
            let Some(s) = self.slot_mut(&w.key) else { continue };
            s.in_flight = Some(tag);
            flying += 1;
            out.push(Effect::Fetch { tag, path: s.path.clone() });
        }
        out
    }

    /// An answer: `status` 200 with a picture that decoded is `ok`; anything else is a refusal (404 `no_picture`, 503 `not_connected`,
    /// 429 `busy`, the helper not answering at 0).
    pub fn arrived(&mut self, tag: u64, status: u16, ok: bool, now: u64) -> Option<Arrival> {
        let s = self.slots.iter_mut().find(|s| s.in_flight == Some(tag))?;
        s.in_flight = None;
        if ok && status == 200 {
            s.pic.last_at = Some(now);
            s.pic.refused = 0;
            s.tries = 0;
            if s.every_ms == 0 {
                s.done = true;
            } else {
                s.next_at = now + s.every_ms;
            }
            return Some(Arrival { key: s.key.clone(), ok: true });
        }
        s.pic.refused += 1;
        s.tries += 1;
        if s.every_ms == 0 {
            if s.tries >= TRIES {
                s.done = true;
            } else {
                s.next_at = now + 700 * u64::from(s.tries);
            }
        } else {
            let wait = if status == 429 || status == 503 { BUSY_MS } else { RETRY_MS };
            s.next_at = now + s.every_ms.max(wait);
        }
        Some(Arrival { key: s.key.clone(), ok: false })
    }

    /// What a key's picture is doing (for a tile's words).
    pub fn pic(&self, key: &str) -> Pic {
        self.slots.iter().find(|s| s.key == key).map_or(Pic::default(), |s| s.pic)
    }

    /// The key the ask with `tag` was for, while it is in flight: what the host stores a decoded picture under.
    pub fn key_of(&self, tag: u64) -> Option<&str> {
        self.slots.iter().find(|s| s.in_flight == Some(tag)).map(|s| s.key.as_str())
    }

    /// Whether a refused picture asked for once has been given up on: its card stays a glyph on a quiet tile.
    pub fn gave_up(&self, key: &str) -> bool {
        self.slots.iter().find(|s| s.key == key).is_some_and(|s| s.done && s.pic.last_at.is_none())
    }

    /// When the next ask of the wanted pictures is due, if any waits for its time.
    pub fn due_at(&self, wants: &[Want]) -> Option<u64> {
        wants
            .iter()
            .filter_map(|w| self.slots.iter().find(|s| s.key == w.key))
            .filter(|s| !s.done && s.in_flight.is_none())
            .map(|s| s.next_at)
            .min()
    }

    /// Nothing is in flight any more as far as the planner is concerned (the viewer closed: its answers are ignored); what was
    /// learned of each picture stays.
    pub fn quiesce(&mut self) {
        for s in &mut self.slots {
            s.in_flight = None;
        }
    }

    /// Forgets every key of this kind (`prefix`): a bubble's pictures of moments the cursor has left.
    pub fn forget_prefix(&mut self, prefix: &str) {
        self.slots.retain(|s| !s.key.starts_with(prefix));
    }

    pub fn in_flight(&self) -> usize {
        self.slots.iter().filter(|s| s.in_flight.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_of(e: &Effect) -> u64 {
        match e {
            Effect::Fetch { tag, .. } => *tag,
            other => panic!("not a fetch: {other:?}"),
        }
    }

    fn path_of(e: &Effect) -> &str {
        match e {
            Effect::Fetch { path, .. } => path,
            other => panic!("not a fetch: {other:?}"),
        }
    }

    #[test]
    fn a_wanted_picture_is_asked_for_at_once_and_the_next_ask_follows_the_last_arrival_by_the_interval() {
        let mut p = Planner::new();
        let wants = [Want::repeating("tile:1", "/v1/cameras/1/frame?w=414&h=232", 200)];
        let fx = p.plan(&wants, 1000, 8);
        assert_eq!(fx.len(), 1);
        assert_eq!(path_of(&fx[0]), "/v1/cameras/1/frame?w=414&h=232");
        assert!(owns(tag_of(&fx[0])));
        assert!(p.plan(&wants, 1010, 8).is_empty(), "one ask a key in flight");
        let a = p.arrived(tag_of(&fx[0]), 200, true, 1150).expect("an answer");
        assert_eq!(a, Arrival { key: "tile:1".into(), ok: true });
        assert_eq!(p.due_at(&wants), Some(1350), "the next ask after the last arrived plus the interval");
        assert!(p.plan(&wants, 1300, 8).is_empty());
        assert_eq!(p.plan(&wants, 1350, 8).len(), 1);
        assert_eq!(p.pic("tile:1"), Pic { last_at: Some(1150), refused: 0 });
    }

    #[test]
    fn a_refused_ask_counts_and_is_asked_again_later_and_a_picture_clears_the_count() {
        let mut p = Planner::new();
        let wants = [Want::repeating("tile:1", "/x", 200)];
        let t = tag_of(&p.plan(&wants, 0, 8)[0]);
        p.arrived(t, 404, false, 100);
        assert_eq!(p.pic("tile:1").refused, 1);
        assert_eq!(p.due_at(&wants), Some(600), "a refusal waits at least 500 ms");
        let t = tag_of(&p.plan(&wants, 600, 8)[0]);
        p.arrived(t, 503, false, 700);
        assert_eq!(p.pic("tile:1").refused, 2, "after two refused asks: No picture");
        assert_eq!(p.due_at(&wants), Some(1700), "a helper that is not connected or busy is left a second");
        let t = tag_of(&p.plan(&wants, 1700, 8)[0]);
        p.arrived(t, 200, true, 1750);
        assert_eq!(p.pic("tile:1"), Pic { last_at: Some(1750), refused: 0 });
    }

    #[test]
    fn a_picture_asked_for_once_is_asked_again_after_a_refusal_a_few_times_and_never_after_it_arrived() {
        let mut p = Planner::new();
        let wants = [Want::once("card:e1", "/v1/cameras/events/e1/thumbnail?w=352&h=198")];
        let t = tag_of(&p.plan(&wants, 0, 8)[0]);
        p.arrived(t, 404, false, 10);
        assert!(!p.gave_up("card:e1"));
        let t = tag_of(&p.plan(&wants, 800, 8)[0]);
        p.arrived(t, 404, false, 810);
        let t = tag_of(&p.plan(&wants, 5000, 8)[0]);
        p.arrived(t, 404, false, 5010);
        assert!(p.gave_up("card:e1"), "three refusals: a glyph on a quiet tile, never a made-up picture");
        assert!(p.plan(&wants, 99_000, 8).is_empty());
        assert_eq!(p.due_at(&wants), None);
        let ok = [Want::once("card:e2", "/y")];
        let t = tag_of(&p.plan(&ok, 0, 8)[0]);
        p.arrived(t, 200, true, 5);
        assert!(p.plan(&ok, 99_000, 8).is_empty(), "arrived: not asked again");
        assert!(!p.gave_up("card:e2"));
    }

    #[test]
    fn at_most_cap_asks_are_in_flight_and_what_is_not_wanted_is_not_asked() {
        let mut p = Planner::new();
        let wants: Vec<Want> = (0..8).map(|i| Want::repeating(&format!("tile:{i}"), &format!("/{i}"), 200)).collect();
        assert_eq!(p.plan(&wants, 0, 3).len(), 3);
        assert_eq!(p.in_flight(), 3);
        assert_eq!(p.plan(&wants, 1, 3).len(), 0, "the cap holds while they are out");
        let only = [wants[0].clone()];
        assert!(p.plan(&only, 2, 3).is_empty(), "tile 0 is in flight already");
    }

    #[test]
    fn a_new_path_for_a_key_is_a_new_ask_and_the_old_answer_is_dropped() {
        let mut p = Planner::new();
        let first = [Want::repeating("bubble", "/thumb?at=1", 0)];
        let t1 = tag_of(&p.plan(&first, 0, 8)[0]);
        let second = [Want::repeating("bubble", "/thumb?at=2", 0)];
        let fx = p.plan(&second, 10, 8);
        assert_eq!(path_of(&fx[0]), "/thumb?at=2");
        assert_eq!(p.arrived(t1, 200, true, 20), None, "the answer to the old moment is not this one's");
        let t2 = tag_of(&fx[0]);
        assert!(p.arrived(t2, 200, true, 30).is_some());
    }

    #[test]
    fn closing_forgets_what_was_in_flight_and_keeps_what_was_learned() {
        let mut p = Planner::new();
        let wants = [Want::repeating("tile:1", "/x", 200)];
        let t = tag_of(&p.plan(&wants, 0, 8)[0]);
        p.arrived(t, 200, true, 5);
        let t2 = tag_of(&p.plan(&wants, 300, 8)[0]);
        p.quiesce();
        assert_eq!(p.arrived(t2, 200, true, 310), None);
        assert_eq!(p.pic("tile:1").last_at, Some(5), "the last picture's age is kept");
        assert_eq!(p.plan(&wants, 400, 8).len(), 1, "and it is asked for again when the viewer opens");
    }

    #[test]
    fn the_tag_range_is_ours_and_wraps() {
        assert!(owns(TAG_FIRST) && owns(TAG_LAST));
        assert!(!owns(0x5E00_0000) && !owns(1));
        let mut p = Planner::new();
        p.counter = TAG_LAST;
        assert_eq!(p.next_tag(), TAG_LAST);
        assert_eq!(p.next_tag(), TAG_FIRST);
    }

    #[test]
    fn forgetting_a_prefix_drops_those_keys_only() {
        let mut p = Planner::new();
        let wants = [Want::once("bubble:1", "/a"), Want::once("tile:1", "/b")];
        p.plan(&wants, 0, 8);
        p.forget_prefix("bubble:");
        assert_eq!(p.in_flight(), 1);
    }
}
