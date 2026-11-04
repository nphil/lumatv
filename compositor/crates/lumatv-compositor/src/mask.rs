//! A moment's blanking of a region of one app's window, for the shell's own overlay to stand in for what the app draws there: LG's channel
//! banner on Live TV (`com.webos.app.livetv`; `inputcommon` draws it into the card's one surface, over the video) while LumaTV's own banner is
//! up. The picture of the window is not drawn inside the rectangles, so what the page holds there shows: with the video's hole under the
//! card, the video.
//!
//! The Rust shell claims it (`POST /v1/mask`) for as long as it is the one that draws the banner, and the claim goes with the shell: a client
//! that is gone, or one that releases, leaves the window as the app draws it. A claim names the app, the rectangles (on the window's own
//! picture), the keys whose press delivered to that app starts the blanking (the channel keys: LG shows its banner after the tuner answers the
//! key, which takes longer than the key's way here, so the banner is blanked before it is drawn) and how long a blanking lasts; an `engage`
//! starts one without a key (Live TV coming to the front). The blanking is timed on the plan's clock, so a test moves it by hand.

use crate::Rect;
use serde_json::Value;

/// A claim names at most this many rectangles, and a blanking lasts at most this long.
pub const MAX_RECTS: usize = 8;
const MAX_HOLD_MS: u64 = 30_000;

const MS: u64 = 1_000_000;

/// What the shell asks of the mask.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Claim(Mask),
    /// Blank for this many milliseconds from the next plan.
    Engage(u64),
    Release,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mask {
    /// The app whose window is blanked.
    pub app: String,
    /// What is blanked, on the window's own picture, in logical pixels.
    pub rects: Vec<Rect>,
    /// The evdev codes whose press, delivered to the app, starts a blanking.
    pub keys: Vec<u32>,
    /// How long a blanking started by a key lasts, in nanoseconds.
    pub hold_ns: u64,
    /// A blanking was asked for (this many nanoseconds of it) and starts at the next plan's clock.
    pending: Option<u64>,
    until_ns: u64,
}

fn rect_of(v: &Value) -> Option<Rect> {
    let n = |k: &str| v.get(k).and_then(Value::as_i64).and_then(|n| i32::try_from(n).ok());
    let r = Rect::new(n("x")?, n("y")?, n("w")?, n("h")?);
    (!r.is_empty() && r.x >= 0 && r.y >= 0).then_some(r)
}

impl Op {
    /// `{"op": "claim", "app", "rects": [{x, y, w, h}], "keys": [codes], "holdMs"}`, `{"op": "engage", "ms"}` or `{"op": "release"}`.
    pub fn parse(body: &Value) -> Result<Op, String> {
        match body.get("op").and_then(Value::as_str) {
            Some("release") => Ok(Op::Release),
            Some("engage") => Ok(Op::Engage(body.get("ms").and_then(Value::as_u64).ok_or("engage needs ms")?.min(MAX_HOLD_MS))),
            Some("claim") => {
                let app = body.get("app").and_then(Value::as_str).filter(|a| !a.is_empty()).ok_or("claim needs an app")?.to_string();
                let rects: Vec<Rect> =
                    body.get("rects").and_then(Value::as_array).ok_or("claim needs rects")?.iter().filter_map(rect_of).take(MAX_RECTS).collect();
                if rects.is_empty() {
                    return Err("claim names no rectangle that is whole".into());
                }
                let keys = body
                    .get("keys")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|k| k.as_u64().and_then(|k| u32::try_from(k).ok())).collect())
                    .unwrap_or_default();
                let hold = body.get("holdMs").and_then(Value::as_u64).unwrap_or(0).min(MAX_HOLD_MS);
                Ok(Op::Claim(Mask { app, rects, keys, hold_ns: hold * MS, pending: None, until_ns: 0 }))
            }
            _ => Err("op is claim, engage or release".into()),
        }
    }
}

impl Mask {
    /// Whether a press of `code` would start a blanking (a cheap test before the key's app is looked up).
    pub fn listens(&self, code: u32) -> bool {
        self.hold_ns > 0 && self.keys.contains(&code)
    }

    /// A press of `code` was delivered to `app`: the blanking starts at the next plan.
    pub fn key(&mut self, app: &str, code: u32) -> bool {
        let yes = self.app == app && self.listens(code);
        if yes {
            self.pending = Some(self.hold_ns);
        }
        yes
    }

    /// A claim that replaces this one (the same app, the banner elsewhere) carries a blanking that is running or asked for over to itself.
    pub fn carry(&mut self, old: &Mask) {
        (self.pending, self.until_ns) = (old.pending, old.until_ns);
    }

    pub fn engage(&mut self, ms: u64) {
        self.pending = Some(ms.min(MAX_HOLD_MS) * MS);
    }

    /// The plan's clock: what was asked for starts now (the later of it and what runs).
    pub fn step(&mut self, now_ns: u64) {
        if let Some(ns) = self.pending.take() {
            self.until_ns = self.until_ns.max(now_ns.saturating_add(ns));
        }
    }

    pub fn active(&self, now_ns: u64) -> bool {
        now_ns < self.until_ns
    }

    /// What `ctl status` says: the claim, and whether it blanks now.
    pub fn report(&self, now_ns: u64) -> Value {
        serde_json::json!({
            "app": self.app,
            "rects": self.rects.iter().map(|r| serde_json::json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})).collect::<Vec<_>>(),
            "keys": self.keys,
            "holdMs": self.hold_ns / MS,
            "active": self.active(now_ns),
        })
    }
}

/// Calls `draw` with each part of `clip` that none of `holes` covers: rectangles that do not overlap (at most 1 + 3 for each hole that cuts it).
pub fn cut(clip: Rect, holes: &[Rect], draw: &mut impl FnMut(Rect)) {
    let Some((hole, rest)) = holes.split_first() else { return draw(clip) };
    let i = clip.intersect(hole);
    if i.is_empty() {
        return cut(clip, rest, draw);
    }
    for part in [
        Rect::new(clip.x, clip.y, clip.w, i.y - clip.y),
        Rect::new(clip.x, i.bottom(), clip.w, clip.bottom() - i.bottom()),
        Rect::new(clip.x, i.y, i.x - clip.x, i.h),
        Rect::new(i.right(), i.y, clip.right() - i.right(), i.h),
    ] {
        if !part.is_empty() {
            cut(part, rest, draw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_claim_names_the_app_its_rectangles_the_keys_and_the_hold_and_a_bad_one_is_refused() {
        let body = json!({"op": "claim", "app": "com.webos.app.livetv", "rects": [{"x": 0, "y": 556, "w": 1512, "h": 168}, {"x": -5, "y": 0, "w": 10, "h": 10}, {"x": 0, "y": 0, "w": 0, "h": 9}],
                          "keys": [402, 403, -1], "holdMs": 6500});
        let Ok(Op::Claim(m)) = Op::parse(&body) else { panic!("a claim") };
        assert_eq!(
            (m.app.as_str(), m.rects.clone(), m.keys.clone(), m.hold_ns),
            ("com.webos.app.livetv", vec![Rect::new(0, 556, 1512, 168)], vec![402, 403], 6500 * MS),
            "a rectangle that is not whole is left out"
        );
        assert!(Op::parse(&json!({"op": "claim", "app": "", "rects": [{"x": 0, "y": 0, "w": 1, "h": 1}]})).is_err());
        assert!(Op::parse(&json!({"op": "claim", "app": "a", "rects": []})).is_err());
        assert_eq!(Op::parse(&json!({"op": "release"})), Ok(Op::Release));
        assert_eq!(Op::parse(&json!({"op": "engage", "ms": 99_999})), Ok(Op::Engage(MAX_HOLD_MS)), "no blanking is for ever");
        assert!(Op::parse(&json!({"op": "engage"})).is_err() && Op::parse(&json!({})).is_err());
    }

    #[test]
    fn a_key_to_the_app_starts_a_blanking_at_the_next_plan_and_it_ends_by_its_own_clock() {
        let Ok(Op::Claim(mut m)) = Op::parse(&json!({"op": "claim", "app": "tv", "rects": [{"x": 0, "y": 0, "w": 9, "h": 9}], "keys": [402], "holdMs": 100}))
        else {
            panic!()
        };
        assert!(!m.active(1) && !m.key("other", 402) && !m.key("tv", 7), "another app's window, another key: nothing");
        assert!(m.key("tv", 402));
        assert!(!m.active(5 * MS), "it starts at the next plan, not before");
        m.step(10 * MS);
        assert!(m.active(10 * MS) && m.active(109 * MS) && !m.active(110 * MS));
        // a second key while it runs carries it on from the later plan; an engage with no key does the same
        m.key("tv", 402);
        m.step(100 * MS);
        assert!(m.active(199 * MS) && !m.active(200 * MS));
        m.engage(50);
        m.step(500 * MS);
        assert!(m.active(549 * MS) && !m.active(550 * MS));
        let mut moved = Mask { app: "tv".into(), ..Mask::default() };
        moved.carry(&m);
        assert!(moved.active(549 * MS) && !moved.active(550 * MS), "a claim that replaces this one keeps the blanking that runs");
        let mut none = Mask { app: "tv".into(), keys: vec![402], ..Mask::default() };
        assert!(!none.key("tv", 402), "a claim with no hold has no key that starts one");
    }

    fn pieces(clip: Rect, holes: &[Rect]) -> Vec<Rect> {
        let mut v = Vec::new();
        cut(clip, holes, &mut |r| v.push(r));
        v
    }

    #[test]
    fn a_rectangle_less_the_blanked_ones_is_the_pieces_that_do_not_overlap_and_add_up() {
        let area = |v: &[Rect]| v.iter().map(|r| i64::from(r.w) * i64::from(r.h)).sum::<i64>();
        let clip = Rect::new(0, 0, 100, 100);
        assert_eq!(pieces(clip, &[]), vec![clip]);
        assert_eq!(pieces(clip, &[Rect::new(200, 0, 5, 5)]), vec![clip], "a hole that is elsewhere cuts nothing");
        let one = pieces(clip, &[Rect::new(10, 20, 30, 40)]);
        assert_eq!(area(&one), 10_000 - 1200);
        assert!(one.iter().all(|r| r.intersect(&Rect::new(10, 20, 30, 40)).is_empty()), "none of it is inside the hole");
        let two = pieces(clip, &[Rect::new(10, 20, 30, 40), Rect::new(25, 30, 50, 10)]);
        assert_eq!(area(&two), 10_000 - 1200 - (50 * 10 - 15 * 10));
        assert!(pieces(clip, &[Rect::new(-5, -5, 200, 200)]).is_empty(), "a hole that covers it leaves nothing");
        for (i, a) in two.iter().enumerate() {
            assert!(two.iter().skip(i + 1).all(|b| a.intersect(b).is_empty()), "the pieces do not overlap");
        }
    }
}
