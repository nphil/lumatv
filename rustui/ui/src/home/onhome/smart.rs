//! The card's behaviour, apart from how it is drawn: which item is in front, what a press does with it, the state the person asked for that
//! is shown before Home Assistant confirms it, the size and the place the owner gives an item.
//!
//! The model of Home owns one [`Smart`] and gives it the topics, the card's box and the keys; what comes out ([`Out`]) is what the model asks
//! of the rest: a control to send, a sheet to open, the camera viewer, a change of the owner's list. Nothing here draws or reads a clock: the
//! time is given.

use super::items::{self, Hold, Item, Kind, Press};
use super::layout::{self, Grid, Metrics, Plan, Size};
use super::Act;
use crate::home::widgets::{PAD_SIDE, PAD_TOP};
use crate::kit::focus::Dir;
use serde_json::{Value, json};
use std::collections::HashMap;
use ui::geometry::Frame;

/// How long what the person asked for is shown in place of what Home Assistant says, once it has said it did it: its state comes by the
/// topic a moment later, and if it does not, the card goes back to what the topic says.
pub const CONFIRM_MS: u64 = 3000;
/// How long it is shown while the control is on its way (the integration answers within 15 s).
pub const AWAIT_MS: u64 = 17_000;
/// A lamp that has no colour of its own shows a warm white: 3000 K.
pub const WARM_WHITE: [u8; 3] = [255, 177, 110];

/// What the person asked for, shown at once.
#[derive(Clone, Debug, PartialEq)]
struct Over {
    id: String,
    on: bool,
    until: u64,
}

/// A thing the model is asked to do because of a key.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Nothing,
    /// The card is left.
    Leave,
    /// A control to send now; `seq` names it, for the answer.
    Act { seq: u64, act: Act },
    /// The sheet of controls of a tile.
    Controls(String),
    /// The question "Unlock <name>?".
    Unlock(String),
    /// The history of a sensor.
    History(String),
    /// The colour sheet of a lamp.
    Colour(String),
    /// The camera viewer, for the camera of the topic with this id.
    Camera(String),
    /// A change of the owner's list (`POST /v1/onhome/picks`).
    Pick(Value),
}

/// The strip of an item's own menu: its size, where it goes, whether it stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strip {
    Size(Size),
    Move,
    Hide,
}

pub const STRIP: [Strip; 5] = [Strip::Size(Size::Small), Strip::Size(Size::Wide), Strip::Size(Size::Large), Strip::Move, Strip::Hide];

impl Strip {
    pub fn glyph(self) -> &'static str {
        match self {
            Strip::Size(s) => s.glyph(),
            Strip::Move => "move",
            Strip::Hide => "eye-off",
        }
    }
}

/// The item menu that is open on the item `id`.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrange {
    pub id: String,
    pub at: Strip,
    /// The item is picked up: the arrows take it from place to place.
    pub moving: bool,
    /// The size the item had when the menu opened: Back puts it back.
    pub was: Size,
}

pub struct Smart {
    base: Vec<Item>,
    items: Vec<Item>,
    metrics: Metrics,
    card: Option<Frame>,
    content: (f32, f32),
    plan: Plan,
    off: usize,
    focus: Option<String>,
    /// OK is down on this item since this time.
    pressed: Option<(String, u64)>,
    hover: Option<String>,
    over: Vec<Over>,
    /// Controls on their way: the number, the item.
    acts: Vec<(u64, String)>,
    seq: u64,
    /// The colour each lamp last showed lit, for one that is switched on again before the topic says.
    colours: HashMap<String, [u8; 3]>,
    pub arrange: Option<Arrange>,
    /// A size shown while the menu is on it, or until the topic has it: the id, the size, until when (none: while the menu is up).
    sized: Vec<(String, Size, Option<u64>)>,
    /// The list's order the owner has made since the topic last said: shown at once.
    ordered: Option<(Vec<String>, u64)>,
}

impl Default for Smart {
    fn default() -> Self {
        Self::new()
    }
}

impl Smart {
    pub fn new() -> Self {
        let metrics = Metrics::nominal(1.0);
        let grid = layout::grid(1.0, 1.0, true, &metrics);
        Self {
            base: Vec::new(),
            items: Vec::new(),
            metrics,
            card: None,
            content: (0.0, 0.0),
            plan: layout::plan(grid, &[]),
            off: 0,
            focus: None,
            pressed: None,
            hover: None,
            over: Vec::new(),
            acts: Vec::new(),
            seq: 0,
            colours: HashMap::new(),
            arrange: None,
            sized: Vec::new(),
            ordered: None,
        }
    }

    // ---- what it holds

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn grid(&self) -> &Grid {
        &self.plan.grid
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// The first stride (column or row) of the view.
    pub fn offset(&self) -> usize {
        self.off
    }

    /// The item in front: its place in the list.
    pub fn focused(&self) -> Option<usize> {
        let id = self.focus.as_deref()?;
        self.items.iter().position(|i| i.id == id)
    }

    pub fn focus_id(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// The item OK is down on, and since when.
    pub fn pressed(&self) -> Option<(usize, u64)> {
        let (id, at) = self.pressed.as_ref()?;
        Some((self.items.iter().position(|i| i.id == *id)?, *at))
    }

    /// The item the pointer is over.
    pub fn hovered(&self) -> Option<usize> {
        let id = self.hover.as_deref()?;
        self.items.iter().position(|i| i.id == id)
    }

    /// Whether a held OK on the item in front does something (a lamp's colours, a fan's speed).
    pub fn can_hold(&self) -> bool {
        self.focused().and_then(|i| self.items.get(i)).is_some_and(|i| i.hold != Hold::Nothing)
    }

    /// The size an item is drawn at: what the list says unless the owner is trying one.
    fn size_of(&self, item: &Item) -> Size {
        self.sized.iter().find(|(id, ..)| *id == item.id).map_or(item.size, |(_, s, _)| *s)
    }

    /// The size an item has in the grid, which may be less than it wants.
    pub fn size_in_grid(&self, index: usize) -> Option<Size> {
        self.plan.placed.get(index).map(|p| p.size)
    }

    // ---- the data

    /// The topics changed: the items are made again. Whether anything changed.
    pub fn set_topics(&mut self, onhome: &Value, cameras: &Value) -> bool {
        let base = items::of_topics(onhome, cameras);
        if base == self.base && self.ordered.is_none() {
            return false;
        }
        for i in &base {
            if let Some(rgb) = i.rgb {
                self.colours.insert(i.id.clone(), rgb);
            }
        }
        // what the person asked for that the topic now says is done is no longer needed
        self.over.retain(|o| base.iter().find(|i| i.id == o.id).is_some_and(|i| i.on != o.on));
        self.sized.retain(|(id, size, until)| until.is_none() || base.iter().find(|i| i.id == *id).is_some_and(|i| i.size != *size));
        if let Some((order, _)) = &self.ordered
            && base.iter().map(|i| i.id.as_str()).eq(order.iter().map(String::as_str))
        {
            self.ordered = None;
        }
        self.base = base;
        self.rebuild();
        true
    }

    /// The card is `card` on the screen, at a text size and with these measures (those of the kit, once the surface has them).
    pub fn set_geometry(&mut self, card: Option<Frame>, metrics: Metrics) -> bool {
        let content = card.map_or((0.0, 0.0), |c| ((c.w - 2.0 * PAD_SIDE).max(10.0), (c.h - 2.0 * PAD_TOP).max(10.0)));
        if card == self.card && metrics == self.metrics && content == self.content {
            return false;
        }
        self.card = card;
        self.metrics = metrics;
        self.content = content;
        self.relayout();
        true
    }

    /// The items as the list says, with what the person asked for put over them.
    fn rebuild(&mut self) {
        let mut items = self.base.clone();
        if let Some((order, _)) = &self.ordered {
            items.sort_by_key(|i| order.iter().position(|o| *o == i.id).unwrap_or(usize::MAX));
        }
        for o in &self.over {
            if let Some(i) = items.iter_mut().find(|i| i.id == o.id) {
                i.on = o.on;
                i.words = if o.on { "On" } else { "Off" }.into();
                i.tone = if o.on { "on" } else { "off" }.into();
                i.rgb = (o.on && i.is_lamp()).then(|| i.rgb.or_else(|| self.colours.get(&i.id).copied()).unwrap_or(WARM_WHITE));
            }
        }
        for (id, size, _) in &self.sized {
            if let Some(i) = items.iter_mut().find(|i| i.id == *id) {
                i.size = *size;
            }
        }
        self.items = items;
        self.relayout();
    }

    fn relayout(&mut self) {
        let wanted: Vec<Size> = self.items.iter().map(|i| i.size).collect();
        self.plan = layout::plan_for(self.content.0, self.content.1, true, &self.metrics, &wanted);
        if let Some(i) = self.focused() {
            self.off = self.plan.offset_for(i, self.off);
        } else {
            self.off = self.off.min(self.plan.max_offset());
        }
    }

    // ---- keys

    /// The card is entered: the item that was in front, else the first.
    pub fn enter(&mut self) {
        // nothing is still pressed and no menu is still open from a visit that was cut short (a sheet over it, the layout changed)
        self.pressed = None;
        self.close_arrange();
        let keep = self.focused().is_some();
        if !keep {
            self.focus = self.items.first().map(|i| i.id.clone());
        }
        if let Some(i) = self.focused() {
            self.off = self.plan.offset_for(i, self.off);
        }
    }

    /// The card is left: nothing is pressed, no menu is open.
    pub fn leave(&mut self) {
        self.pressed = None;
        self.close_arrange();
    }

    fn focus_index(&mut self, i: usize) {
        self.focus = self.items.get(i).map(|it| it.id.clone());
        self.off = self.plan.offset_for(i, self.off);
    }

    /// A D-pad key moves to the item that lies that way. Whether it moved.
    pub fn go(&mut self, dir: Dir) -> bool {
        let Some(from) = self.focused() else {
            self.enter();
            return self.focused().is_some();
        };
        match self.plan.step(from, dir) {
            Some(to) => {
                self.focus_index(to);
                true
            }
            None => false,
        }
    }

    /// OK went down on the item in front. While its menu is open the menu has the keys: nothing under it is pressed.
    pub fn press_down(&mut self, now: u64) {
        if self.pressed.is_none() && self.arrange.is_none()
            && let Some(id) = self.focus.clone().filter(|id| self.items.iter().any(|i| i.id == *id))
        {
            self.pressed = Some((id, now));
        }
    }

    /// OK went up without being held: what it does.
    pub fn press_up(&mut self, now: u64) -> Out {
        let Some((id, _)) = self.pressed.take() else { return Out::Nothing };
        let Some(i) = self.items.iter().position(|it| it.id == id) else { return Out::Nothing };
        self.press(i, now)
    }

    /// The press of OK on the item `i`.
    pub fn press(&mut self, i: usize, now: u64) -> Out {
        self.focus_index(i);
        let item = self.items[i].clone();
        match item.press {
            Press::Toggle => self.toggle(&item, now),
            Press::Run => self.act(&item, "run", Value::Null, now),
            Press::Lock => self.act(&item, "lock", Value::Null, now),
            Press::Unlock => Out::Unlock(item.id),
            Press::Controls => Out::Controls(item.id),
            Press::History => Out::History(item.id),
            Press::Camera => Out::Camera(items::camera_of(&item.id).unwrap_or("").to_owned()),
        }
    }

    /// OK held as long as a hold is: what the item has under it, if anything.
    pub fn hold(&mut self) -> Out {
        let Some((id, _)) = self.pressed.take() else { return Out::Nothing };
        let Some(item) = self.item(&id).cloned() else { return Out::Nothing };
        match item.hold {
            Hold::Colour => Out::Colour(item.id),
            Hold::Controls => Out::Controls(item.id),
            Hold::Nothing => Out::Nothing,
        }
    }

    /// The press is over without an OK (the card was left, a sheet took the keys).
    pub fn cancel_press(&mut self) {
        self.pressed = None;
    }

    fn act(&mut self, item: &Item, verb: &'static str, value: Value, _now: u64) -> Out {
        self.seq += 1;
        self.acts.push((self.seq, item.id.clone()));
        Out::Act { seq: self.seq, act: Act { id: item.id.clone(), verb, value } }
    }

    /// Switches a lamp, a switch or a fan: the new state is shown at once and stands until the topic says it, or the control fails.
    fn toggle(&mut self, item: &Item, now: u64) -> Out {
        let on = !item.on;
        self.over.retain(|o| o.id != item.id);
        self.over.push(Over { id: item.id.clone(), on, until: now + AWAIT_MS });
        let out = self.act(item, "toggle", Value::Null, now);
        self.rebuild();
        out
    }

    /// The answer to the control `seq`: it was done (`error` none), or it was refused or failed, in words that Home says. A control that
    /// failed is taken back: the card shows what the topic says again.
    pub fn answered(&mut self, seq: u64, error: Option<&str>, now: u64) -> bool {
        let Some(at) = self.acts.iter().position(|(s, _)| *s == seq) else { return false };
        let (_, id) = self.acts.remove(at);
        match error {
            None => {
                for o in self.over.iter_mut().filter(|o| o.id == id) {
                    o.until = now + CONFIRM_MS;
                }
            }
            Some(_) => {
                self.over.retain(|o| o.id != id);
                self.rebuild();
            }
        }
        true
    }

    /// OK went down on the item `i` with the pointer: it is in front and pressed.
    pub fn press_item(&mut self, i: usize, now: u64) {
        self.focus_index(i);
        self.pressed = self.items.get(i).map(|it| (it.id.clone(), now));
    }

    /// The owner's change was refused: the size and the place that were shown at once are not kept.
    pub fn forget_arrangement(&mut self) {
        self.sized.retain(|(_, _, until)| until.is_none());
        self.ordered = None;
        self.rebuild();
    }

    /// Time passes: what was shown in place of the topic and has outlasted its time goes. Whether anything changed.
    pub fn tick(&mut self, now: u64) -> bool {
        let mut changed = false;
        let before = self.over.len() + self.sized.iter().filter(|(.., u)| u.is_some()).count() + usize::from(self.ordered.is_some());
        self.over.retain(|o| now < o.until);
        self.sized.retain(|(.., until)| until.is_none_or(|u| now < u));
        if self.ordered.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.ordered = None;
        }
        let after = self.over.len() + self.sized.iter().filter(|(.., u)| u.is_some()).count() + usize::from(self.ordered.is_some());
        if after != before {
            self.rebuild();
            changed = true;
        }
        changed
    }

    /// When the next thing that is shown for a time ends.
    pub fn due_at(&self) -> Option<u64> {
        let over = self.over.iter().map(|o| o.until);
        let sized = self.sized.iter().filter_map(|(.., u)| *u);
        let ordered = self.ordered.iter().map(|(_, u)| *u);
        over.chain(sized).chain(ordered).min()
    }

    // ---- the pointer

    fn content_origin(&self) -> Option<(f32, f32)> {
        self.card.map(|c| (c.x + PAD_SIDE, c.y + PAD_TOP))
    }

    /// The item under the point (`x`, `y`) of the screen.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let (ox, oy) = self.content_origin()?;
        self.plan.hit(x - ox, y - oy, self.off)
    }

    /// Whether the point (`x`, `y`) of the screen is on the card.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.card.is_some_and(|c| c.contains(x, y))
    }

    /// The pointer is over `index`, or over nothing. Whether the cue moved.
    pub fn hover_item(&mut self, index: Option<usize>) -> bool {
        let id = index.and_then(|i| self.items.get(i)).map(|i| i.id.clone());
        let changed = id != self.hover;
        self.hover = id;
        changed
    }

    /// The wheel: the view moves a stride without moving the item in front.
    pub fn scroll(&mut self, by: i32) -> bool {
        let max = self.plan.max_offset();
        let to = (self.off as i64 + i64::from(by)).clamp(0, max as i64) as usize;
        let changed = to != self.off;
        self.off = to;
        changed
    }

    /// The point of the screen the item `i` has its first pixel at, at rest: for the pointer's hit test and for the surface.
    pub fn screen_rect(&self, i: usize) -> Option<Frame> {
        let (ox, oy) = self.content_origin()?;
        let r = self.plan.rect(i, self.off)?;
        Some(Frame::new(ox + r.x, oy + r.y, r.w, r.h))
    }

    // ---- the item menu

    /// The menu of the item in front: its size, its place, whether it stays.
    pub fn open_arrange(&mut self) -> bool {
        let Some(i) = self.focused() else { return false };
        // a camera is chosen where the cameras are, not on the owner's list of things
        if self.items[i].kind == Kind::Camera {
            return false;
        }
        let item = &self.items[i];
        let size = self.size_of(item);
        self.pressed = None;
        self.arrange = Some(Arrange { id: item.id.clone(), at: Strip::Size(size), moving: false, was: size });
        true
    }

    /// The menu goes: a size that was only being tried goes with it (one that was chosen has its own entry).
    pub fn close_arrange(&mut self) {
        if let Some(a) = self.arrange.take() {
            self.sized.retain(|(id, _, until)| !(*id == a.id && until.is_none()));
            self.rebuild();
        }
    }

    fn arrange_key(&mut self, name: &str, now: u64) -> Option<Out> {
        let a = self.arrange.clone()?;
        if a.moving {
            return Some(self.moving_key(&a, name, now));
        }
        let at = STRIP.iter().position(|s| *s == a.at).unwrap_or(0);
        match name {
            "left" | "right" => {
                let to = if name == "left" { at.saturating_sub(1) } else { (at + 1).min(STRIP.len() - 1) };
                if to != at {
                    if let Some(arr) = self.arrange.as_mut() {
                        arr.at = STRIP[to];
                    }
                    // a size is tried as soon as it is in front
                    if let Strip::Size(s) = STRIP[to] {
                        self.sized.retain(|(id, ..)| *id != a.id);
                        self.sized.push((a.id.clone(), s, None));
                        self.rebuild();
                    }
                }
                Some(Out::Nothing)
            }
            "up" | "down" => Some(Out::Nothing),
            "back" | "menu" => {
                self.close_arrange();
                Some(Out::Nothing)
            }
            "ok" => Some(match a.at {
                Strip::Size(s) => {
                    self.sized.retain(|(id, ..)| *id != a.id);
                    self.sized.push((a.id.clone(), s, Some(now + AWAIT_MS)));
                    self.arrange = None;
                    self.rebuild();
                    Out::Pick(json!({"size": a.id, "to": s.id()}))
                }
                Strip::Move => {
                    // picked up at the size it has, not the one that was in front of the menu
                    self.sized.retain(|(id, _, until)| !(*id == a.id && until.is_none()));
                    if let Some(arr) = self.arrange.as_mut() {
                        arr.moving = true;
                    }
                    self.rebuild();
                    Out::Nothing
                }
                Strip::Hide => {
                    let id = a.id.clone();
                    let at = self.focused().unwrap_or(0);
                    self.close_arrange();
                    self.base.retain(|i| i.id != id);
                    self.rebuild();
                    let next = at.min(self.items.len().saturating_sub(1));
                    self.focus_index(next);
                    if self.items.is_empty() {
                        self.focus = None;
                    }
                    Out::Pick(json!({"hide": id}))
                }
            }),
            _ => Some(Out::Nothing),
        }
    }

    /// The item is picked up: Left and Up take it one place earlier, Right and Down one later; OK and Back put it down where it is.
    fn moving_key(&mut self, a: &Arrange, name: &str, now: u64) -> Out {
        let Some(from) = self.items.iter().position(|i| i.id == a.id) else {
            self.arrange = None;
            return Out::Nothing;
        };
        let cols = self.plan.grid.cols.max(1);
        let step: i64 = match name {
            "left" => -1,
            "right" => 1,
            "up" => -(cols as i64),
            "down" => cols as i64,
            "ok" | "back" | "menu" => {
                self.close_arrange();
                return Out::Nothing;
            }
            _ => return Out::Nothing,
        };
        let to = (from as i64 + step).clamp(0, self.items.len() as i64 - 1) as usize;
        if to == from {
            return Out::Nothing;
        }
        // the list the owner has is the tiles' only: the cameras at its head are not in it
        let cameras = self.items.iter().take_while(|i| i.kind == Kind::Camera).count();
        if from < cameras || to < cameras {
            return Out::Nothing;
        }
        let mut order: Vec<String> = self.items.iter().map(|i| i.id.clone()).collect();
        let moved = order.remove(from);
        order.insert(to, moved);
        self.ordered = Some((order, now + AWAIT_MS));
        self.rebuild();
        self.focus_index(to);
        Out::Pick(json!({"move": a.id, "to": to - cameras}))
    }

    /// A key while the card is entered. `None` when it is not one the card has, which leaves the card.
    pub fn key(&mut self, name: &str, now: u64) -> Option<Out> {
        if self.arrange.is_some() {
            return self.arrange_key(name, now);
        }
        let dir = match name {
            "up" => Some(Dir::Up),
            "down" => Some(Dir::Down),
            "left" => Some(Dir::Left),
            "right" => Some(Dir::Right),
            _ => None,
        };
        match name {
            "back" => None,
            "menu" => {
                self.open_arrange();
                Some(Out::Nothing)
            }
            "ok" => Some(if self.pressed.is_some() { self.press_up(now) } else { self.press_up_now(now) }),
            _ => {
                self.go(dir?);
                Some(Out::Nothing)
            }
        }
    }

    /// OK as one key (a key that went down and up in one step: the pointer's click, a test).
    fn press_up_now(&mut self, now: u64) -> Out {
        self.press_down(now);
        self.press_up(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(id: &str, on: bool) -> Value {
        json!({"id": id, "name": id, "group": "lights", "words": if on { "On" } else { "Off" }, "tone": if on { "on" } else { "off" }, "icon": "lightbulb", "available": true, "can": ["toggle"], "verbs": ["turn_on", "turn_off", "toggle"],
               "ctl": {"on": on, "dimmable": true, "brightness": if on { json!(60) } else { Value::Null }, "colour": true, "rgb": if on { json!([255, 0, 128]) } else { Value::Null }, "kelvin": null, "effects": []}})
    }

    fn sensor() -> Value {
        json!({"id": "sensor.t", "name": "Hall", "group": "sensors", "words": "21.4\u{b0}C", "tone": "info", "icon": "thermometer", "available": true, "can": [], "verbs": [], "ctl": {"kind": "number", "value": 21.4, "unit": "\u{b0}C", "class": "temperature"}})
    }

    fn smart(tiles: Value, cameras: Value) -> Smart {
        let mut s = Smart::new();
        // a 6 by 5 card of the editor's grid
        s.set_geometry(Some(Frame::new(100.0, 400.0, 886.0, 265.0)), Metrics::nominal(1.0));
        s.set_topics(&json!({"state": "connected", "tiles": tiles}), &cameras);
        s
    }

    #[test]
    fn entering_puts_the_first_item_in_front_and_the_keys_move_among_them_without_wrapping() {
        let mut s = smart(json!([lamp("light.a", true), lamp("light.b", false), sensor()]), Value::Null);
        assert_eq!(s.focused(), None);
        s.enter();
        assert_eq!(s.focused(), Some(0));
        assert!(s.go(Dir::Right) && s.focused() == Some(1));
        assert!(s.go(Dir::Right) && s.focused() == Some(2));
        assert!(!s.go(Dir::Right), "the end holds");
        assert!(!s.go(Dir::Up) && !s.go(Dir::Down), "one row");
        assert!(s.go(Dir::Left) && s.focused() == Some(1));
        // entered again, the item that was in front is
        s.leave();
        s.enter();
        assert_eq!(s.focused(), Some(1));
        // Back is not a key the card has: it leaves
        assert_eq!(s.key("back", 0), None);
    }

    #[test]
    fn a_toggle_is_shown_at_once_and_stands_until_the_topic_says_it() {
        let mut s = smart(json!([lamp("light.a", false)]), Value::Null);
        s.enter();
        s.press_down(1000);
        assert_eq!(s.pressed().map(|p| p.0), Some(0), "down is a press before it is a toggle");
        assert!(!s.item("light.a").unwrap().on, "nothing has changed yet");
        let Out::Act { seq, act } = s.press_up(1090) else { panic!("a control") };
        assert_eq!((act.id.as_str(), act.verb), ("light.a", "toggle"));
        let lamp_now = s.item("light.a").unwrap();
        assert_eq!((lamp_now.on, lamp_now.words.as_str(), lamp_now.rgb), (true, "On", Some(WARM_WHITE)), "shown on in the same step as the key");
        // Home Assistant did it: the state stands a moment more, until the topic says it
        assert!(s.answered(seq, None, 1200));
        assert!(s.item("light.a").unwrap().on);
        s.set_topics(&json!({"tiles": [lamp("light.a", true)]}), &Value::Null);
        assert_eq!(s.item("light.a").unwrap().rgb, Some([255, 0, 128]), "the topic's own colour replaces the guess");
        assert!(s.due_at().is_none(), "nothing is left to wait for");
        // and a topic that never says it: the card goes back to what is
        let mut s = smart(json!([lamp("light.a", false)]), Value::Null);
        s.enter();
        let Out::Act { seq, .. } = s.key("ok", 1000).unwrap() else { panic!() };
        s.answered(seq, None, 1100);
        assert!(s.item("light.a").unwrap().on);
        assert_eq!(s.due_at(), Some(1100 + CONFIRM_MS));
        assert!(s.tick(1100 + CONFIRM_MS));
        assert!(!s.item("light.a").unwrap().on, "eased back to what Home Assistant says");
    }

    #[test]
    fn a_control_that_fails_is_taken_back_and_says_why() {
        let mut s = smart(json!([lamp("light.a", true), lamp("light.b", false)]), Value::Null);
        s.enter();
        let Out::Act { seq: a, .. } = s.key("ok", 1000).unwrap() else { panic!() };
        s.go(Dir::Right);
        let Out::Act { seq: b, .. } = s.key("ok", 1010).unwrap() else { panic!() };
        assert_eq!((s.item("light.a").unwrap().on, s.item("light.b").unwrap().on), (false, true), "both shown at once");
        // the first fails; the second is not touched by it
        assert!(s.answered(a, Some("Home Assistant cannot reach it right now"), 1200));
        assert_eq!((s.item("light.a").unwrap().on, s.item("light.b").unwrap().on), (true, true));
        assert!(!s.answered(a, None, 1300), "an answer is taken once");
        s.answered(b, None, 1300);
    }

    #[test]
    fn what_a_press_opens_depends_on_what_the_item_is() {
        let cams = json!({"cameras": [{"id": "front", "name": "Front door", "online": true}]});
        let lock = json!({"id": "lock.f", "name": "Front", "group": "locks", "words": "Locked", "tone": "ok", "icon": "lock", "available": true, "can": ["unlock"], "verbs": ["lock"], "ctl": {"state": "locked"}});
        let mut s = smart(json!([sensor(), lock, lamp("light.a", true)]), cams);
        s.enter();
        assert_eq!(s.key("ok", 10), Some(Out::Camera("front".into())), "the camera opens the viewer");
        s.go(Dir::Right);
        assert_eq!(s.key("ok", 20), Some(Out::History("sensor.t".into())), "a sensor opens its history");
        s.go(Dir::Right);
        assert_eq!(s.key("ok", 30), Some(Out::Unlock("lock.f".into())), "a locked lock asks first");
        s.go(Dir::Right);
        // a held OK on a lamp opens its colour sheet, and its release is not a toggle
        s.press_down(100);
        assert_eq!(s.hold(), Out::Colour("light.a".into()));
        assert_eq!(s.press_up(900), Out::Nothing);
        assert!(s.item("light.a").unwrap().on, "the hold did not switch it");
    }

    #[test]
    fn the_pointer_finds_the_item_under_it_and_the_wheel_moves_the_view() {
        let tiles: Vec<Value> = (0..12).map(|i| lamp(&format!("light.l{i}"), i % 2 == 0)).collect();
        let mut s = smart(Value::Array(tiles), Value::Null);
        let r = s.screen_rect(1).unwrap();
        assert_eq!(s.hit(r.x + 4.0, r.y + 4.0), Some(1));
        assert_eq!(s.hit(5.0, 5.0), None);
        let cols = s.grid().cols;
        assert!(s.grid().axis == layout::Axis::Horizontal && s.plan().max_offset() == 12 - cols);
        assert!(s.scroll(1) && s.offset() == 1);
        let r = s.screen_rect(1).unwrap();
        assert!(r.x < s.screen_rect(2).unwrap().x);
        assert_eq!(s.hit(r.x + 4.0, r.y + 4.0), Some(1), "the point finds the item where the view is now");
        assert!(!s.scroll(-2) || s.offset() == 0);
        assert!(!s.scroll(-1), "the start holds");
        assert!(s.hover_item(Some(3)) && !s.hover_item(Some(3)) && s.hovered() == Some(3));
    }

    #[test]
    fn the_view_follows_the_item_in_front_by_whole_strides() {
        let tiles: Vec<Value> = (0..12).map(|i| lamp(&format!("light.l{i}"), false)).collect();
        let mut s = smart(Value::Array(tiles), Value::Null);
        s.enter();
        let cols = s.grid().cols;
        for _ in 0..cols - 1 {
            s.go(Dir::Right);
        }
        assert_eq!(s.offset(), 0, "the last one in view needs no scroll");
        s.go(Dir::Right);
        assert_eq!(s.offset(), 1);
        for _ in 0..20 {
            s.go(Dir::Right);
        }
        assert_eq!((s.focused(), s.offset()), (Some(11), 12 - cols));
        for _ in 0..20 {
            s.go(Dir::Left);
        }
        assert_eq!((s.focused(), s.offset()), (Some(0), 0));
    }

    #[test]
    fn the_item_menu_tries_a_size_as_it_is_reached_and_keeps_it_on_ok_and_back_puts_it_back() {
        let mut s = smart(json!([lamp("light.a", true), lamp("light.b", false)]), Value::Null);
        s.enter();
        assert_eq!(s.key("menu", 0), Some(Out::Nothing));
        assert_eq!(s.arrange.as_ref().map(|a| a.at), Some(Strip::Size(Size::Small)));
        s.key("right", 1);
        assert_eq!(s.arrange.as_ref().map(|a| a.at), Some(Strip::Size(Size::Wide)));
        assert_eq!(s.plan().placed[0].size, Size::Wide, "the preview is the grid, live");
        // Back puts it back
        s.key("back", 2);
        assert!(s.arrange.is_none());
        assert_eq!(s.plan().placed[0].size, Size::Small);
        // OK keeps it, and says so to the helper
        s.key("menu", 3);
        s.key("right", 4);
        let out = s.key("ok", 5).unwrap();
        assert_eq!(out, Out::Pick(json!({"size": "light.a", "to": "wide"})));
        assert_eq!(s.plan().placed[0].size, Size::Wide, "kept until the topic says it");
        let mut wide = lamp("light.a", true);
        wide["size"] = json!("wide");
        s.set_topics(&json!({"tiles": [wide, lamp("light.b", false)]}), &Value::Null);
        assert_eq!(s.plan().placed[0].size, Size::Wide);
        assert!(s.sized.is_empty(), "the topic has it now");
    }

    #[test]
    fn ok_in_the_item_menu_is_no_press_on_the_item_under_it_and_a_press_does_not_outlive_the_menu_that_opened_on_it() {
        let mut s = smart(json!([lamp("light.a", true), lamp("light.b", false)]), Value::Null);
        s.enter();
        s.key("menu", 0);
        s.press_down(1);
        assert_eq!(s.pressed(), None, "the menu has the keys: nothing under it is pressed");
        s.key("ok", 2);
        assert_eq!(s.pressed(), None);
        // a press under way when the menu opens goes with it, so that no stale press is made later on another item
        s.press_down(10);
        assert!(s.pressed().is_some());
        s.key("menu", 11);
        assert_eq!(s.pressed(), None);
        s.key("back", 12);
        s.key("right", 13);
        assert_eq!(s.press_up(14), Out::Nothing, "nothing is pressed: no stale press toggles the lamp that was in front");
    }

    #[test]
    fn a_picked_up_item_goes_from_place_to_place_and_hide_takes_it_off_the_card() {
        let mut s = smart(json!([lamp("light.a", true), lamp("light.b", false), lamp("light.c", false)]), Value::Null);
        s.enter();
        s.key("menu", 0);
        for _ in 0..3 {
            s.key("right", 1);
        }
        assert_eq!(s.arrange.as_ref().map(|a| a.at), Some(Strip::Move));
        s.key("ok", 2);
        assert!(s.arrange.as_ref().is_some_and(|a| a.moving));
        assert_eq!(s.key("right", 3), Some(Out::Pick(json!({"move": "light.a", "to": 1}))));
        assert_eq!(s.items().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["light.b", "light.a", "light.c"], "at once");
        assert_eq!(s.focused(), Some(1), "the focus goes with it");
        assert_eq!(s.key("right", 4), Some(Out::Pick(json!({"move": "light.a", "to": 2}))));
        assert_eq!(s.key("right", 5), Some(Out::Nothing), "the end holds");
        assert_eq!(s.key("ok", 6), Some(Out::Nothing));
        assert!(s.arrange.is_none(), "put down where it is");
        // hide
        s.key("menu", 7);
        for _ in 0..4 {
            s.key("right", 8);
        }
        assert_eq!(s.arrange.as_ref().map(|a| a.at), Some(Strip::Hide));
        assert_eq!(s.key("ok", 9), Some(Out::Pick(json!({"hide": "light.a"}))));
        assert_eq!(s.items().len(), 2);
        assert!(s.focused().is_some(), "the focus lands on a neighbour");
    }

    #[test]
    fn the_cameras_at_the_head_are_not_in_the_owners_list_and_are_not_moved() {
        let cams = json!({"cameras": [{"id": "front", "online": true}]});
        let mut s = smart(json!([lamp("light.a", true), lamp("light.b", false)]), cams);
        s.enter();
        s.go(Dir::Right);
        s.key("menu", 0);
        for _ in 0..3 {
            s.key("right", 1);
        }
        s.key("ok", 2);
        assert_eq!(s.key("left", 3), Some(Out::Nothing), "a lamp does not go before the camera");
        s.key("back", 4);
        s.key("menu", 5);
        for _ in 0..3 {
            s.key("right", 6);
        }
        s.key("ok", 7);
        assert_eq!(s.key("right", 8), Some(Out::Pick(json!({"move": "light.a", "to": 1}))), "a place in the tiles' list, which has no camera in it");
    }
}
