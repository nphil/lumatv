//! A list that opens next to something (`docs/mocks/16-context-menu`, `17-power-menu`, `33-pointer-menu`; `panels/ContextMenu.qml` of the QML
//! shell): the options on a dock tile (Open, Move, Hide from the dock, Rename, Uninstall), the power menu, the menu under the Magic
//! Remote's pointer. One menu that is given a spec when it opens:
//!
//! ```text
//! spec = { title, sub, icon (Lucide) | iconUrl (a picture's file), x, y (where it opens, in the 1920x1080 scene),
//!          anchor: { x, y, width, height, place } (instead of x, y: opens beside that rectangle, level with its bottom edge, on the left of
//!          it when it does not fit on the right; place "below" opens under it, left edges level, above it when it does not fit),
//!          raised: { x, y, width, height, iconUrl, plate, inset, radius } (the thing the menu belongs to, drawn above the scrim so it
//!          stays lit: a dock tile or a library icon),
//!          items: [{ id, icon, label, detail, hint, danger, disabled, options: [...], index }], hint, width, scrim }
//! ```
//!
//! An item with `options` is a segmented row (the sleep timer): Left and Right move inside it. Up and Down move, OK picks, Back closes.
//! Danger items (Uninstall) are in the danger colour and are asked about twice by whoever receives them. The card stays on the screen:
//! it is moved back inside the safe area when the given place would put it outside. What was picked is told to the owner as the event
//! `menu.picked {id, index}` and the menu closes: the owner decides what follows.

use super::life::Life;
use super::{K, Out};
use serde_json::{Value, json};
use ui::geometry::Frame;

/// The card keeps this far from the edges of the screen.
pub const MARGIN: f32 = 48.0;
pub const DEFAULT_WIDTH: f32 = 520.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Right,
    Below,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub frame: Frame,
    pub place: Place,
}

/// The thing the menu belongs to, lit above the scrim.
#[derive(Clone, Debug, PartialEq)]
pub struct Raised {
    pub frame: Frame,
    pub icon_url: String,
    /// The colour behind the picture (`#RRGGBB`), none when it has no plate.
    pub plate: Option<String>,
    pub inset: f32,
    pub radius: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub icon: String,
    pub label: String,
    pub detail: String,
    pub hint: String,
    pub danger: bool,
    pub disabled: bool,
    pub options: Vec<String>,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub title: String,
    pub sub: String,
    pub icon: String,
    pub icon_url: String,
    pub at: Option<(Option<f32>, Option<f32>)>,
    pub anchor: Option<Anchor>,
    pub raised: Option<Raised>,
    pub items: Vec<Item>,
    pub hint: String,
    pub width: f32,
    /// A scrim behind the card (the pointer's menu has none).
    pub scrim: bool,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn num(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(Value::as_f64).map(|n| n as f32)
}

impl Spec {
    pub fn from_json(v: &Value) -> Spec {
        let anchor = v.get("anchor").filter(|a| a.is_object()).map(|a| Anchor {
            frame: Frame::new(num(a, "x").unwrap_or(0.0), num(a, "y").unwrap_or(0.0), num(a, "width").unwrap_or(0.0), num(a, "height").unwrap_or(0.0)),
            place: if a.get("place").and_then(Value::as_str) == Some("below") { Place::Below } else { Place::Right },
        });
        let raised = v.get("raised").filter(|r| r.is_object()).map(|r| Raised {
            frame: Frame::new(num(r, "x").unwrap_or(0.0), num(r, "y").unwrap_or(0.0), num(r, "width").unwrap_or(0.0), num(r, "height").unwrap_or(0.0)),
            icon_url: text(r, "iconUrl"),
            plate: r.get("plate").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
            inset: num(r, "inset").unwrap_or(0.0),
            radius: num(r, "radius"),
        });
        let items = v
            .get("items")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|i| Item {
                        id: text(i, "id"),
                        icon: text(i, "icon"),
                        label: text(i, "label"),
                        detail: text(i, "detail"),
                        hint: text(i, "hint"),
                        danger: i.get("danger").and_then(Value::as_bool) == Some(true),
                        disabled: i.get("disabled").and_then(Value::as_bool) == Some(true),
                        options: i.get("options").and_then(Value::as_array).map(|o| o.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
                        index: i.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let at = (v.get("x").is_some() || v.get("y").is_some()).then(|| (num(v, "x"), num(v, "y")));
        Spec {
            title: text(v, "title"),
            sub: text(v, "sub"),
            icon: text(v, "icon"),
            icon_url: text(v, "iconUrl"),
            at,
            anchor,
            raised,
            items,
            hint: text(v, "hint"),
            width: num(v, "width").unwrap_or(DEFAULT_WIDTH),
            scrim: v.get("scrim").and_then(Value::as_bool) != Some(false),
        }
    }

    pub fn has_head(&self) -> bool {
        !self.title.is_empty() || !self.sub.is_empty()
    }

    /// Where the card of `w` by `h` goes: at the given place or beside its anchor, moved back inside the safe margin.
    pub fn place_card(&self, w: f32, h: f32, gap: f32, gap_small: f32) -> (f32, f32) {
        let (want_x, want_y) = match &self.anchor {
            None => {
                let (x, y) = self.at.unwrap_or((None, None));
                (x.unwrap_or(((1920.0 - w) / 2.0).round()), y.unwrap_or(((1080.0 - h) / 2.0).round()))
            }
            Some(a) => {
                let f = a.frame;
                match a.place {
                    Place::Below => (f.x, if f.y + f.h + gap_small + h + MARGIN <= 1080.0 { f.y + f.h + gap_small } else { f.y - gap_small - h }),
                    Place::Right => (if f.x + f.w + gap + w + MARGIN <= 1920.0 { f.x + f.w + gap } else { f.x - gap - w }, f.y + f.h - h),
                }
            }
        };
        (want_x.min(1920.0 - MARGIN - w).max(MARGIN), want_y.min(1080.0 - MARGIN - h).max(MARGIN))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub spec: Spec,
    pub index: usize,
    pub seg: usize,
    pub over_video: bool,
}

#[derive(Debug, Default)]
pub struct Menu {
    pub life: Life,
    spec: Option<Spec>,
    index: usize,
    seg: usize,
    up: bool,
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_up(&self) -> bool {
        self.up
    }

    pub fn spec(&self) -> Option<&Spec> {
        self.spec.as_ref()
    }

    /// The menu opens on the first item that is not disabled.
    pub fn open(&mut self, spec: Spec) {
        self.index = spec.items.iter().position(|i| !i.disabled).unwrap_or(0);
        self.seg = spec.items.get(self.index).filter(|i| !i.options.is_empty()).map_or(0, |i| i.index);
        self.spec = Some(spec);
        self.up = true;
    }

    pub fn close(&mut self) {
        self.up = false;
    }

    fn pick(&mut self) -> Out {
        let Some(it) = self.spec.as_ref().and_then(|s| s.items.get(self.index)).filter(|i| !i.disabled) else { return Out::default() };
        let mut out = Out::close();
        out.events.push(("menu.picked".into(), json!({"id": it.id, "index": self.seg})));
        out
    }

    pub fn key(&mut self, k: K) -> Out {
        let Some(spec) = self.spec.as_ref() else { return Out::close() };
        match k {
            K::Up | K::Down => {
                let d: i32 = if k == K::Up { -1 } else { 1 };
                let mut n = self.index as i32 + d;
                while n >= 0 && (n as usize) < spec.items.len() && spec.items[n as usize].disabled {
                    n += d;
                }
                if n >= 0 && (n as usize) < spec.items.len() {
                    self.index = n as usize;
                    self.seg = if spec.items[self.index].options.is_empty() { 0 } else { spec.items[self.index].index };
                }
            }
            K::Left | K::Right => {
                if let Some(it) = spec.items.get(self.index).filter(|i| !i.options.is_empty()) {
                    let d: i32 = if k == K::Left { -1 } else { 1 };
                    self.seg = (self.seg as i32 + d).clamp(0, it.options.len() as i32 - 1) as usize;
                }
            }
            K::Ok => return self.pick(),
            K::Back => return Out::close(),
            _ => {}
        }
        Out::default()
    }

    /// A click on item `i`: it takes the focus and is picked (a segmented row picks the segment under the pointer, `seg`).
    pub fn click(&mut self, i: usize, seg: Option<usize>) -> Out {
        let Some(spec) = self.spec.as_ref() else { return Out::default() };
        if spec.items.get(i).is_none_or(|it| it.disabled) {
            return Out::default();
        }
        self.index = i;
        self.seg = match (seg, spec.items[i].options.len()) {
            (Some(s), n) if n > 0 => s.min(n - 1),
            _ => if spec.items[i].options.is_empty() { 0 } else { spec.items[i].index },
        };
        self.pick()
    }

    /// A click outside the card is Back.
    pub fn click_outside(&mut self) -> Out {
        Out::close()
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn seg(&self) -> usize {
        self.seg
    }

    pub fn view(&self, over_video: bool) -> Option<View> {
        self.spec.as_ref().map(|s| View { up: self.up, spec: s.clone(), index: self.index, seg: self.seg, over_video })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dock_menu() -> Spec {
        Spec::from_json(&json!({
            "title": "Crunchyroll", "sub": "Streaming", "icon": "play",
            "anchor": {"x": 700, "y": 600, "width": 160, "height": 160},
            "raised": {"x": 700, "y": 600, "width": 160, "height": 160, "iconUrl": "/media/cr.png", "plate": "#fff", "inset": 12},
            "items": [
                {"id": "open", "icon": "play", "label": "Open"},
                {"id": "move", "icon": "move", "label": "Move", "disabled": true},
                {"id": "unpin", "icon": "eye-off", "label": "Hide from the dock", "detail": "Stays in Apps"},
                {"id": "uninstall", "icon": "trash-2", "label": "Uninstall", "danger": true}
            ],
            "hint": "Back closes"
        }))
    }

    #[test]
    fn it_opens_on_the_first_item_that_is_not_disabled_and_up_and_down_skip_the_disabled_ones() {
        let mut m = Menu::new();
        m.open(dock_menu());
        assert_eq!(m.index(), 0);
        m.key(K::Down);
        assert_eq!(m.index(), 2, "Move is disabled: the focus goes past it");
        m.key(K::Down);
        m.key(K::Down);
        assert_eq!(m.index(), 3, "the end holds");
        m.key(K::Up);
        m.key(K::Up);
        assert_eq!(m.index(), 0);
        m.key(K::Up);
        assert_eq!(m.index(), 0);
    }

    #[test]
    fn ok_picks_the_item_tells_the_owner_and_closes_and_back_closes_without_a_pick() {
        let mut m = Menu::new();
        m.open(dock_menu());
        m.key(K::Down);
        let out = m.key(K::Ok);
        assert!(out.close);
        assert_eq!(out.events, vec![("menu.picked".to_string(), json!({"id": "unpin", "index": 0}))]);
        m.open(dock_menu());
        let out = m.key(K::Back);
        assert!(out.close && out.events.is_empty());
    }

    #[test]
    fn a_segmented_row_moves_inside_itself_and_picks_its_segment() {
        let mut m = Menu::new();
        m.open(Spec::from_json(&json!({"title": "Power", "items": [
            {"id": "off", "label": "Turn off"},
            {"id": "sleep", "label": "Sleep timer", "options": ["Off", "10 min", "30 min"], "index": 1}
        ]})));
        m.key(K::Down);
        assert_eq!(m.seg(), 1, "it opens on the current choice");
        m.key(K::Right);
        m.key(K::Right);
        assert_eq!(m.seg(), 2, "the end holds");
        m.key(K::Left);
        m.key(K::Left);
        m.key(K::Left);
        assert_eq!(m.seg(), 0);
        m.key(K::Right);
        let out = m.key(K::Ok);
        assert_eq!(out.events, vec![("menu.picked".to_string(), json!({"id": "sleep", "index": 1}))]);
        m.open(Spec::from_json(&json!({"items": [{"id": "a", "label": "A"}]})));
        m.key(K::Right);
        assert_eq!(m.seg(), 0, "Left and Right mean nothing on an item without segments");
    }

    #[test]
    fn a_click_picks_the_item_under_the_pointer_unless_it_is_disabled_and_outside_is_back() {
        let mut m = Menu::new();
        m.open(dock_menu());
        assert!(m.click(1, None).events.is_empty(), "a disabled item swallows the click");
        let out = m.click(3, None);
        assert_eq!(out.events, vec![("menu.picked".to_string(), json!({"id": "uninstall", "index": 0}))]);
        assert!(m.click_outside().close);
    }

    #[test]
    fn the_card_opens_beside_its_anchor_or_where_it_is_told_and_stays_inside_the_safe_margin() {
        let spec = dock_menu();
        // right of the tile, level with its bottom edge
        let (x, y) = spec.place_card(520.0, 400.0, 16.0, 4.0);
        assert_eq!((x, y), (700.0 + 160.0 + 16.0, 600.0 + 160.0 - 400.0));
        // no room on the right: on its left
        let mut near = dock_menu();
        near.anchor.as_mut().unwrap().frame = Frame::new(1500.0, 300.0, 160.0, 160.0);
        let (x, _) = near.place_card(520.0, 400.0, 16.0, 4.0);
        assert_eq!(x, 1500.0 - 16.0 - 520.0);
        // below, and above when it does not fit under
        let mut below = dock_menu();
        below.anchor = Some(Anchor { frame: Frame::new(300.0, 200.0, 160.0, 160.0), place: Place::Below });
        assert_eq!(below.place_card(520.0, 300.0, 16.0, 4.0), (300.0, 364.0));
        below.anchor = Some(Anchor { frame: Frame::new(300.0, 800.0, 160.0, 160.0), place: Place::Below });
        assert_eq!(below.place_card(520.0, 300.0, 16.0, 4.0), (300.0, 800.0 - 4.0 - 300.0));
        // given x and y: used, then kept inside; none: centred
        let at = Spec::from_json(&json!({"x": 1800, "y": 20, "items": []}));
        assert_eq!(at.place_card(520.0, 300.0, 16.0, 4.0), (1920.0 - 48.0 - 520.0, 48.0));
        let centred = Spec::from_json(&json!({"items": []}));
        assert_eq!(centred.place_card(520.0, 300.0, 16.0, 4.0), (700.0, 390.0));
    }

    #[test]
    fn a_spec_reads_what_the_qml_menu_read() {
        let s = dock_menu();
        assert!(s.has_head());
        assert_eq!((s.width, s.scrim), (DEFAULT_WIDTH, true));
        assert_eq!(s.items.len(), 4);
        assert!(s.items[3].danger && s.items[1].disabled);
        assert_eq!(s.items[2].detail, "Stays in Apps");
        let r = s.raised.as_ref().unwrap();
        assert!(r.plate.as_deref() == Some("#fff") && r.inset == 12.0 && r.icon_url == "/media/cr.png");
        assert!(!Spec::from_json(&json!({"scrim": false, "items": []})).scrim);
        assert!(!Spec::from_json(&json!({"items": []})).has_head());
    }
}
