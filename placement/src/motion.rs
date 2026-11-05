//! Moving an item: one arrow press, a pointer's drop. Fractions of the room an item has, so a step is the same few pixels wherever the item is.

use crate::model::{Anchor, Frame, Item, Placement, Registry, Sizes, clamp};

/// One press of an arrow moves an item this many pixels.
pub const STEP: i32 = 12;
/// A pointer within this many pixels of a stop lands on it.
pub const SNAP_PX: i32 = 18;
/// The corners, the thirds and the middle: where an item stops as it moves.
pub const STOPS: [f64; 5] = [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0];
/// A stop this close is where the item is (a placement keeps four decimals).
const EPS: f64 = 0.001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn parse(name: &str) -> Option<Dir> {
        match name {
            "left" => Some(Dir::Left),
            "right" => Some(Dir::Right),
            "up" => Some(Dir::Up),
            "down" => Some(Dir::Down),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::Left => "left",
            Dir::Right => "right",
            Dir::Up => "up",
            Dir::Down => "down",
        }
    }
}

/// A press of an arrow: whether the key is held (the key repeat) and for how many presses, so that a held key glides over the stops and
/// speeds up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    pub repeat: bool,
    pub run: u32,
}

/// Where a press took an item. `moved` is false where there is nowhere to go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub placement: Placement,
    pub moved: bool,
}

/// One arrow press along one axis: `f` is where the item is (0 to 1 of its room), `room` the room in pixels. A tap moves a fine step and stops
/// on the next corner, third or middle when one lies in the way or within half a step beyond; a held key (`repeat`) glides over them. Always
/// inside 0 to 1.
pub fn axis(f: f64, dir: f64, room: f64, px: f64, repeat: bool) -> f64 {
    if room <= 0.0 {
        return f;
    }
    let d = px / room;
    let mut next = clamp(f + dir * d, 0.0, 1.0);
    if !repeat {
        let mut hit: Option<f64> = None;
        for s in STOPS {
            let ahead = if dir > 0.0 {
                s > f + EPS && s <= next + d / 2.0
            } else {
                s < f - EPS && s >= next - d / 2.0
            };
            if ahead && hit.is_none_or(|h| (s - f).abs() < (h - f).abs()) {
                hit = Some(s);
            }
        }
        if let Some(h) = hit {
            next = h;
        }
    }
    next
}

/// How far a press moves an item: a held key's step grows the longer it is held.
pub fn pixels(run: u32) -> f64 {
    f64::from(STEP)
        * if run > 12 {
            6.0
        } else if run > 5 {
            3.0
        } else {
            1.0
        }
}

/// A pointer's value within a few pixels of a corner, a third or the middle is that stop.
pub fn snap(f: f64, room_px: f64, px: i32) -> f64 {
    if room_px <= 0.0 {
        return f;
    }
    STOPS
        .into_iter()
        .find(|s| (f - s).abs() * room_px <= f64::from(px))
        .unwrap_or(f)
}

impl Registry {
    /// One press of `dir` on item `id` at placement `p`; `None` for an id the registry does not know. An item that lives on an edge slides
    /// along it and, past its end, turns the corner onto the next edge (a press of its own, not a held key's); a side edge's opposite key flips
    /// it to the other side.
    pub fn step(
        &self,
        id: &str,
        p: &Placement,
        dir: Dir,
        sizes: &Sizes,
        f: &Frame,
        press: Press,
    ) -> Option<Step> {
        let it = self.item(id)?;
        let px = pixels(press.run);
        let z = it.zone.rect(f);
        if !it.edges.is_empty() {
            return Some(edge_step(it, p, dir, sizes, &z, px, press.repeat));
        }
        let s = sizes.at(it, p.anchor);
        let (mut fx, mut fy) = p.fractions();
        match dir {
            Dir::Left => fx = axis(fx, -1.0, f64::from(z.w - s.w), px, press.repeat),
            Dir::Right => fx = axis(fx, 1.0, f64::from(z.w - s.w), px, press.repeat),
            Dir::Up => fy = axis(fy, -1.0, f64::from(z.h - s.h), px, press.repeat),
            Dir::Down => fy = axis(fy, 1.0, f64::from(z.h - s.h), px, press.repeat),
        }
        let np = self.from_fractions(it, fx, fy).with_full(p.full);
        Some(Step {
            placement: np,
            moved: !np.same(p),
        })
    }

    /// The placement of an item whose middle is at (`cx`, `cy`), for a pointer that drags it: a value within a few pixels of a corner, a third or
    /// the middle is that stop. An edge item goes to the nearest of its edges and slides along it.
    pub fn at_point(
        &self,
        id: &str,
        cx: f64,
        cy: f64,
        sizes: &Sizes,
        f: &Frame,
        p: &Placement,
    ) -> Option<Placement> {
        let it = self.item(id)?;
        let z = it.zone.rect(f);
        let (zx, zy, zw, zh) = (
            f64::from(z.x),
            f64::from(z.y),
            f64::from(z.w),
            f64::from(z.h),
        );
        if !it.edges.is_empty() {
            let (mut best, mut bd) = (it.edges[0], f64::MAX);
            for e in it.edges {
                let (ax, ay) = e.fractions();
                // the distance to the edge's own line: along the edge is free
                let d = if ax == 0.5 {
                    0.0
                } else {
                    (cx - (zx + ax * zw)).abs()
                } + if ay == 0.5 {
                    0.0
                } else {
                    (cy - (zy + ay * zh)).abs()
                };
                if d < bd {
                    bd = d;
                    best = *e;
                }
            }
            let sz = sizes.at(it, best);
            let vertical = best != Anchor::Bottom;
            let room = f64::from(if vertical { z.h - sz.h } else { z.w - sz.w });
            let t = if room > 0.0 {
                (if vertical {
                    cy - f64::from(sz.h) / 2.0 - zy
                } else {
                    cx - f64::from(sz.w) / 2.0 - zx
                }) / room
            } else {
                0.5
            };
            return Some(Placement::on_edge(
                best,
                snap(clamp(t, 0.0, 1.0), room, SNAP_PX),
            ));
        }
        let s = sizes.at(it, p.anchor);
        let (rw, rh) = (f64::from(z.w - s.w), f64::from(z.h - s.h));
        let fx = if rw > 0.0 {
            clamp((cx - f64::from(s.w) / 2.0 - zx) / rw, 0.0, 1.0)
        } else {
            0.5
        };
        let fy = if rh > 0.0 {
            clamp((cy - f64::from(s.h) / 2.0 - zy) / rh, 0.0, 1.0)
        } else {
            0.5
        };
        Some(self.from_fractions(it, snap(fx, rw, SNAP_PX), snap(fy, rh, SNAP_PX)).with_full(p.full))
    }
}

fn edge_step(
    it: &Item,
    p: &Placement,
    dir: Dir,
    sizes: &Sizes,
    z: &crate::model::Rect,
    px: f64,
    repeat: bool,
) -> Step {
    let stay = Step {
        placement: *p,
        moved: false,
    };
    let edge = p.anchor;
    let a = p.along();
    let s = sizes.at(it, edge);
    let vertical = edge != Anchor::Bottom;
    let along = if vertical {
        match dir {
            Dir::Up => -1.0,
            Dir::Down => 1.0,
            _ => 0.0,
        }
    } else {
        match dir {
            Dir::Left => -1.0,
            Dir::Right => 1.0,
            _ => 0.0,
        }
    };
    if along != 0.0 {
        let room = f64::from(if vertical { z.h - s.h } else { z.w - s.w });
        let a2 = axis(a, along, room, px, repeat);
        if a2 != a {
            return Step {
                placement: Placement::on_edge(edge, a2),
                moved: true,
            };
        }
        // at the end of the edge: one more press turns the corner, when the next edge is one of its own; a held key does not
        if repeat {
            return stay;
        }
        let turn = if edge == Anchor::Bottom {
            Some(if along < 0.0 {
                Anchor::Left
            } else {
                Anchor::Right
            })
        } else if along > 0.0 {
            Some(Anchor::Bottom)
        } else {
            None
        };
        if let Some(t) = turn {
            if it.edges.contains(&t) {
                let at = if t == Anchor::Bottom && edge == Anchor::Left {
                    0.0
                } else {
                    1.0
                };
                return Step {
                    placement: Placement::on_edge(t, at),
                    moved: true,
                };
            }
        }
        return stay;
    }
    // across: a side edge's other key flips it to the opposite side, at the same height
    if (edge == Anchor::Left && dir == Dir::Right) || (edge == Anchor::Right && dir == Dir::Left) {
        let to = if edge == Anchor::Left {
            Anchor::Right
        } else {
            Anchor::Left
        };
        if it.edges.contains(&to) {
            return Step {
                placement: Placement::on_edge(to, a),
                moved: true,
            };
        }
    }
    stay
}
