//! The overlay cards drawn (`panels/OverlayCard.qml`, the mocks `docs/mocks/overlays`): a glass card 560 wide with a tile for what it is, the words
//! (or a score), who said it and the keys under them. The card takes no focus and has no buttons: OK and Back act on it, and the hint says what OK
//! does (the record's first action, or the drawer). Where the column starts is the placement crate's (`layout.overlays.cards`, under the camera
//! window where the two meet); a card's height follows its words, so the second is under the first at its real height and the queue is told each
//! height before it decides how many fit.
//!
//! A card is made when it comes and dropped when it has gone (the scene's budget): it slides in from a touch to the right while it fades in and out
//! again quicker; one whose words change is made again in place, one that moves up to the first place glides there.

use super::cards::{self, Entry, Score};
use super::parts::{Sheet, hint_into, hint_width, tile};
use crate::kit::hint::{Action, Hint, Key};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Kit, wrap};
use compositor_protocol::Rgba;
use lumatv_placement::{Frame as PlacementFrame, Rect as PlaceRect, Registry, Size, Sizes, User};
use lumatv_tokens::TypeRole;
use serde_json::{Value, json};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::text::Weight;

/// The tile's side: 56 at 100 % text, growing with the type but not as fast (a tile twice as big at 200 % is a billboard).
fn lead(kit: &Kit) -> f32 {
    (56.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round()
}
const DOT: f32 = 12.0;
const SLIDE: f32 = 24.0;

/// One piece of a card's plan, placed relative to the card's corner.
enum Item {
    Text { at: Frame, style: Style, text: String },
    Dot { at: (f32, f32) },
    Hint { at: (f32, f32), h: f32, hint: Hint },
}

/// What a card is made of at a width: its height and each piece's place.
struct Plan {
    height: f32,
    items: Vec<Item>,
    icon: String,
    tone: String,
}

fn icon_name(kit: &mut Kit, e: &Entry) -> String {
    if e.cls == "alert" && !e.icon_name.is_empty() {
        let n = super::plugins::Hub::icon(&e.icon_name);
        if n != "plug" && kit.icons.has(&n) {
            return n;
        }
    }
    e.icon.clone()
}

/// The two keys' hints: OK where it does something Back does not (a record's action or the drawer); OK alone where it only dismisses.
pub fn hints_of(e: &Entry, drawer_ready: bool) -> Vec<Hint> {
    let ok = cards::ok_of(e, drawer_ready);
    if ok.label == "Dismiss" && ok.action.is_none() {
        vec![Hint::key(Key::Ok, Action::Close)]
    } else if ok.action.is_none() {
        vec![Hint::key(Key::Ok, Action::Open), Hint::key(Key::Back, Action::Close)]
    } else {
        vec![Hint::labelled(Key::Ok, &ok.label), Hint::key(Key::Back, Action::Close)]
    }
}

fn meta_of(s: &Score) -> String {
    match (s.league.is_empty(), s.status.is_empty()) {
        (false, false) => format!("{}  \u{b7}  {}", s.league, s.status),
        _ => format!("{}{}", s.league, s.status),
    }
}

fn plan(kit: &mut Kit, e: &Entry, w: f32, drawer_ready: bool) -> Plan {
    let pad = kit.space(5);
    let who_x = pad + lead(kit) + kit.space(4);
    let who_w = w - who_x - pad;
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let mut items = Vec::new();
    let who_h;
    if let Some(s) = &e.score {
        let label = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
        let live_w = if s.live { width_of(kit, label, "Live") + kit.space(2) + DOT } else { 0.0 };
        let meta_w = who_w - if s.live { live_w + kit.space(3) } else { 0.0 };
        let meta = wrap::lines(kit, label, &meta_of(s), meta_w, 2);
        let lh = kit.line(TypeRole::Label);
        for (i, l) in meta.iter().enumerate() {
            items.push(Item::Text { at: Frame::new(who_x, pad + i as f32 * lh, meta_w, lh), style: label, text: l.clone() });
        }
        if s.live {
            items.push(Item::Dot { at: (who_x + who_w - live_w, pad + ((lh - DOT) / 2.0).round()) });
            items.push(Item::Text { at: Frame::new(who_x + who_w - live_w + DOT + kit.space(2), pad, live_w - DOT - kit.space(2), lh), style: label, text: "Live".into() });
        }
        let meta_h = meta.len() as f32 * lh;
        let (a, h) = (s.away.score.parse::<i64>().unwrap_or(0), s.home.score.parse::<i64>().unwrap_or(0));
        let (name_h, score_h) = (kit.line(TypeRole::Body), kit.line(TypeRole::Title));
        let row_h = name_h.max(score_h);
        let mut y = pad + meta_h + kit.space(2);
        for (side, ahead) in [(&s.away, a > h), (&s.home, h > a)] {
            let name_style = role_style(kit, TypeRole::Body, Some(if ahead { Weight::SemiBold } else { Weight::Medium }), if ahead { ink } else { ink2 }, Align::Left);
            let score_style = role_style(kit, TypeRole::Title, None, ink, Align::Right);
            let sw = width_of(kit, score_style, &side.score);
            let name = wrap::elide(kit, name_style, &side.name, who_w - sw - kit.space(4));
            items.push(Item::Text { at: Frame::new(who_x, y + ((row_h - name_h) / 2.0).round(), who_w - sw - kit.space(4), name_h), style: name_style, text: name });
            items.push(Item::Text { at: Frame::new(who_x + who_w - sw, y + ((row_h - score_h) / 2.0).round(), sw, score_h), style: score_style, text: side.score.clone() });
            y += row_h;
        }
        who_h = y - pad;
    } else {
        let title_style = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
        let body_style = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
        let title = wrap::lines(kit, title_style, &e.title, who_w, 2);
        let body = if e.body.is_empty() { Vec::new() } else { wrap::lines(kit, body_style, &e.body, who_w, 2) };
        let (th, bh) = (kit.line(TypeRole::Body), kit.line(TypeRole::Label));
        for (i, l) in title.iter().enumerate() {
            items.push(Item::Text { at: Frame::new(who_x, pad + i as f32 * th, who_w, th), style: title_style, text: l.clone() });
        }
        let title_h = title.len() as f32 * th;
        let body_top = pad + title_h + kit.space(1);
        for (i, l) in body.iter().enumerate() {
            items.push(Item::Text { at: Frame::new(who_x, body_top + i as f32 * bh, who_w, bh), style: body_style, text: l.clone() });
        }
        who_h = title_h + if body.is_empty() { 0.0 } else { kit.space(1) + body.len() as f32 * bh };
    }
    // under the words: who said it, and the keys: on one line when they fit, else the keys take a line of their own, and at large text each
    // key takes one (a key's words are never cut)
    let foot_w = w - 2.0 * pad;
    let foot_y = pad + lead(kit).max(who_h) + kit.space(3);
    let caption = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let hints = hints_of(e, drawer_ready);
    let hint_h = crate::kit::hint::glyph_side(kit, TypeRole::Caption).max(kit.line(TypeRole::Caption) as u32) as f32;
    let widths: Vec<f32> = hints.iter().map(|h| hint_width(kit, h, TypeRole::Caption)).collect();
    let sp = kit.space(5);
    let keys_w = widths.iter().sum::<f32>() + sp * widths.len().saturating_sub(1) as f32;
    let source = if e.score.is_some() { String::new() } else { e.source.clone() };
    let has_src = !source.is_empty();
    let src_w = if has_src { width_of(kit, caption, &source).min(foot_w) } else { 0.0 };
    let one_line = if has_src { src_w + sp + keys_w <= foot_w } else { keys_w <= foot_w };
    let stacked = keys_w > foot_w;
    let src_h = kit.line(TypeRole::Caption);
    let foot_h = (if has_src && !one_line { src_h + kit.space(1) } else { 0.0 })
        + if stacked { hints.len() as f32 * hint_h + hints.len().saturating_sub(1) as f32 * kit.space(1) } else { hint_h };
    if has_src {
        let src = wrap::elide(kit, caption, &source, foot_w);
        items.push(Item::Text { at: Frame::new(pad, foot_y, foot_w, src_h), style: caption, text: src });
    }
    let mut y = foot_y + if has_src && !one_line { src_h + kit.space(1) } else { 0.0 };
    let mut x = if one_line && !stacked { pad + foot_w - keys_w } else { pad };
    for (hint, hw) in hints.into_iter().zip(widths) {
        items.push(Item::Hint { at: (x, y), h: hint_h, hint });
        if stacked {
            y += hint_h + kit.space(1);
        } else {
            x += hw + sp;
        }
    }
    Plan { height: foot_y + foot_h + pad, items, icon: icon_name(kit, e), tone: e.tone.clone() }
}

fn darker(c: Rgba, by: f32) -> Rgba {
    let s = |v: u8| (f32::from(v) / by).round() as u8;
    Rgba(s(c.0), s(c.1), s(c.2), c.3)
}

/// The tone's glyph colour, pushed where the material needs it to read (the light accent is dim on the glass over the worst frame).
fn tone_ink(kit: &Kit, tone: &str) -> Rgba {
    let look = &kit.ui.look;
    match tone {
        "danger" => look.colour(PaletteRole::Danger),
        "warning" => look.colour(PaletteRole::Warning),
        _ if kit.mode == ui::lucent::palette::Mode::Light => darker(look.accent(), 1.6),
        _ => look.accent(),
    }
}

/// What makes two renderings of a card the same card.
fn signature(e: &Entry, drawer_ready: bool, w: f32) -> String {
    format!("{:?}", (&e.key, &e.title, &e.body, &e.source, &e.tone, &e.icon, &e.icon_name, &e.score, e.actions.first(), drawer_ready, w as i32))
}

struct Word {
    text: String,
    frame: Frame,
    style: Style,
}

struct Slot {
    key: String,
    sig: String,
    generation: u64,
    over_video: bool,
    sheet: Sheet,
    frame: Frame,
    paint: Rgba,
    words: Vec<Word>,
}

/// How the column is drawn: the card width and the gap between cards, the camera window's size while it is up, whether OK can open the drawer,
/// whether an app's picture is under it.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub width: f32,
    pub gap: f32,
    pub popup: Option<Size>,
    pub drawer_ready: bool,
    pub over_video: bool,
}

pub struct CardsSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    slots: Vec<Slot>,
    leaving: Vec<(Sheet, u64)>,
}

impl CardsSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, slots: Vec::new(), leaving: Vec::new() }
    }

    /// The owner's places changed (`layout.overlays`).
    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    /// A card's height as it will be drawn: what the queue needs to know how many fit.
    pub fn height_of(kit: &mut Kit, e: &Entry, width: f32, drawer_ready: bool) -> f32 {
        plan(kit, e, width, drawer_ready).height
    }

    /// Where the column of cards of `size` is: beside the camera window (its size, when it is up) where the two meet. `waits`: it would leave its
    /// zone there, so the cards wait for the window to go.
    pub fn column(&self, popup: Option<Size>, size: Size) -> (PlaceRect, bool) {
        let mut sizes = Sizes::new();
        sizes.set("cards", size);
        match popup {
            Some(p) => {
                sizes.set("popup", p);
                let laid = self.registry.layout_all(&self.user, &self.frame, &sizes, None, false);
                laid.get("cards").map_or((PlaceRect::new(0, 0, size.w, size.h), false), |l| (l.rect, l.waits))
            }
            None => (self.registry.rect_of("cards", &self.user, &self.frame, size).unwrap_or(PlaceRect::new(0, 0, size.w, size.h)), false),
        }
    }

    /// Where the first card starts: the column's top for a card of a typical height.
    pub fn top(&self, popup: Option<Size>, width: f32) -> f64 {
        f64::from(self.column(popup, Size::new(width as i32, 150)).0.y)
    }

    /// Fits the screen to `shown` (top first): cards that came are made and slide in, those that went slide out, the rest stay or glide.
    /// `popup`: the camera window's size while it is up. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, shown: &[Entry], col: &Column, now_ms: u64) -> std::io::Result<bool> {
        let Column { width, gap, popup, drawer_ready, over_video } = *col;
        // what is made for another look or another text size is made again
        let stale: Vec<usize> = (0..self.slots.len()).filter(|i| self.slots[*i].generation != kit.generation() || self.slots[*i].over_video != over_video).collect();
        for i in stale.into_iter().rev() {
            let s = self.slots.remove(i);
            drop(s);
        }
        let plans: Vec<Plan> = shown.iter().map(|e| plan(kit, e, width, drawer_ready)).collect();
        let total = plans.iter().map(|p| p.height).sum::<f32>() + gap * plans.len().saturating_sub(1) as f32;
        let (rect, _) = self.column(popup, Size::new(width as i32, total.ceil().max(1.0) as i32));
        // cards that are no longer shown go
        let slide = if kit.ui.look.reduced_motion { 0.0 } else { SLIDE };
        let mut i = 0;
        while i < self.slots.len() {
            if shown.iter().any(|e| e.key == self.slots[i].key) {
                i += 1;
                continue;
            }
            let mut s = self.slots.remove(i);
            s.sheet.slide_out(kit, slide);
            self.leaving.push((s.sheet, now_ms + u64::from(kit.beat_ms(crate::kit::motion::Beat::Rest)) + 40));
        }
        let mut y = rect.y as f32;
        let x = rect.x as f32;
        for (e, p) in shown.iter().zip(&plans) {
            let sig = signature(e, drawer_ready, width);
            let at = (x, y);
            let existing = self.slots.iter().position(|s| s.key == e.key);
            match existing {
                Some(k) if self.slots[k].sig == sig => {
                    let s = &mut self.slots[k];
                    if (s.frame.x, s.frame.y) != at {
                        let (dx, dy) = (at.0 - s.frame.x, at.1 - s.frame.y);
                        s.sheet.shift(kit, dx, dy, false);
                        s.frame.x = at.0;
                        s.frame.y = at.1;
                        for w in &mut s.words {
                            w.frame.x += dx;
                            w.frame.y += dy;
                        }
                    }
                }
                other => {
                    let came = other.is_none();
                    if let Some(k) = other {
                        // its words changed: made again where it is, with no new arrival
                        let old = self.slots.remove(k);
                        drop(old);
                    }
                    let slot = Self::build(kit, self.z, e, p, width, at, sig, over_video)?;
                    let mut slot = slot;
                    slot.sheet.slide_in(kit, slide, !came);
                    self.slots.push(slot);
                }
            }
            y += p.height + gap;
        }
        // the slots keep the order of the cards
        self.slots.sort_by_key(|s| shown.iter().position(|e| e.key == s.key).unwrap_or(usize::MAX));
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the cards could not be drawn whole"));
        }
        Ok(!self.slots.is_empty() || !self.leaving.is_empty())
    }

    #[allow(clippy::too_many_arguments)]
    fn build(kit: &mut Kit, z: i32, e: &Entry, p: &Plan, w: f32, at: (f32, f32), sig: String, over_video: bool) -> std::io::Result<Slot> {
        let frame = Frame::new(at.0, at.1, w, p.height);
        let mut sheet = Sheet::new();
        sheet.pane(Pane::new(kit, frame, z, PaneKind::Card, over_video));
        let pad = kit.space(5);
        let ink2 = kit.ui.look.ink2();
        let (plate_layer, glyph) = tile(kit, lead(kit), z + 1, &p.icon, false, tone_ink(kit, &p.tone))?;
        sheet.shape(plate_layer, (at.0 + pad, at.1 + pad));
        if let Some(g) = glyph {
            sheet.image(g, super::parts::tile_glyph_at((at.0 + pad, at.1 + pad), lead(kit)));
        } else if let Some(fallback) = icon::layer(kit, "bell", (lead(kit) * 0.55).round() as u32, tone_ink(kit, &p.tone), z + 2, super::parts::HIDDEN)? {
            sheet.image(fallback, super::parts::tile_glyph_at((at.0 + pad, at.1 + pad), lead(kit)));
        }
        let mut words = Vec::new();
        for item in &p.items {
            match item {
                Item::Text { at: f, style, text } => {
                    let frame = Frame::new(at.0 + f.x, at.1 + f.y, f.w, f.h);
                    sheet.text(Text::new(kit, frame, z + 2, *style, text)?);
                    words.push(Word { text: text.clone(), frame, style: *style });
                }
                Item::Dot { at: d } => {
                    let live = kit.ui.look.colour(PaletteRole::Live);
                    let dot = plate::shape(&kit.ui, Frame::new(0.0, 0.0, DOT, DOT), z + 2, plate::solid(live, DOT / 2.0));
                    sheet.shape(dot, (at.0 + d.0, at.1 + d.1));
                }
                Item::Hint { at: h, h: hh, hint } => {
                    hint_into(kit, &mut sheet, hint, TypeRole::Caption, at.0 + h.0, at.1 + h.1, *hh, z + 2, ink2)?;
                }
            }
        }
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        Ok(Slot { key: e.key.clone(), sig, generation: kit.generation(), over_video, sheet, frame, paint, words })
    }

    /// Drops the cards that have finished sliding out. Returns whether one went.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let n = self.leaving.len();
        self.leaving.retain(|(_, until)| *until > now_ms);
        n != self.leaving.len()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving.iter().map(|(_, until)| *until).min()
    }

    pub fn is_up(&self) -> bool {
        !self.slots.is_empty()
    }

    /// The column's rectangle while cards are up: what the mute mark keeps clear of.
    pub fn rect(&self) -> Option<PlaceRect> {
        let first = self.slots.first()?;
        let last = self.slots.last()?;
        Some(PlaceRect::new(first.frame.x as i32, first.frame.y as i32, first.frame.w as i32, (last.frame.y + last.frame.h - first.frame.y) as i32))
    }

    pub fn geometry(&self) -> Value {
        json!(self.slots.iter().map(|s| json!({"key": s.key, "x": s.frame.x, "y": s.frame.y, "w": s.frame.w, "h": s.frame.h})).collect::<Vec<_>>())
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        for (i, s) in self.slots.iter().enumerate() {
            let panel = lint.panel(&format!("card {i}"), s.frame, s.paint, true);
            for (j, w) in s.words.iter().enumerate() {
                lint.word(kit, &format!("card {i} words {j}"), &w.text, w.frame, w.style, Some(panel));
            }
        }
    }

    /// The layers the cards hold (the budget).
    pub fn layers(&self) -> usize {
        self.slots.iter().map(|s| s.sheet.layers()).sum::<usize>() + self.leaving.iter().map(|(s, _)| s.layers()).sum::<usize>()
    }

    pub fn restyle(&mut self) {
        self.slots.clear();
        self.leaving.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::system::overlay::Overlay;
    use crate::system::plugins::Note;
    use ui::lucent::palette::Mode;

    const T0: u64 = 1_800_000_000_000;

    fn note(id: &str, kind: &str, title: &str, body: &str, actions: Value, score: Value) -> Note {
        Note {
            plugin_id: "p".into(),
            plugin: "Kitchen Show".into(),
            id: id.into(),
            channel: kind.into(),
            channel_title: "Kitchen Show".into(),
            channel_icon: String::new(),
            revision: 1,
            received_at: 1.0,
            expires_at: 0.0,
            title: title.into(),
            body: body.into(),
            actions: actions.as_array().cloned().unwrap_or_default(),
            kind: kind.into(),
            level: String::new(),
            privacy: String::new(),
            score,
            importance: String::new(),
            channel_kind: String::new(),
        }
    }

    fn two_cards(kit: &mut Kit) -> Vec<Entry> {
        let mut o = Overlay::new(&json!({}));
        o.posted(
            &[
                note("t", "timer", "Pasta", "Time's up", json!([{"id": "stop", "label": "Stop", "action": "stop"}]), Value::Null),
                note("b", "doorbell", "Front door", "Someone is at the door with a parcel that needs a signature, and a long line of words to wrap", json!([]), Value::Null),
            ],
            T0,
        );
        o.refresh(134.0, &mut |e| f64::from(CardsSurface::height_of(kit, e, 560.0, false)), T0);
        o.shown().to_vec()
    }

    fn col(popup: Option<Size>) -> Column {
        Column { width: 560.0, gap: 16.0, popup, drawer_ready: false, over_video: false }
    }

    fn lint_of(kit: &mut Kit, s: &CardsSurface) -> Vec<String> {
        let mut lint = Lint::new();
        s.feed_lint(kit, &mut lint);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    fn rig(mode: Mode, scale: f32) -> (ui::testing::Harness, Kit, CardsSurface) {
        let (h, kit) = testkit::kit_in(mode, scale);
        let layout: Value = serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap();
        (h, kit, CardsSurface::new(1060, &json!({}), &layout))
    }

    #[test]
    fn two_cards_are_a_column_flush_at_home_s_right_edge_with_the_second_under_the_first_and_read_clean_in_both_materials_and_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit, mut s) = rig(mode, scale);
                let shown = two_cards(&mut kit);
                // at 200 % text the second card would end below the limit, so it waits: the first always shows
                assert_eq!(shown.len(), if scale == 1.0 { 2 } else { 1 }, "how many fit above 810 at {scale}");
                assert!(s.apply(&mut kit, &shown, &col(None), 1000).unwrap());
                let g = s.geometry();
                let cards = g.as_array().unwrap();
                let a = &cards[0];
                assert_eq!(a["x"].as_f64().unwrap() + a["w"].as_f64().unwrap(), 1856.0, "flush at x 1856: {a}");
                assert_eq!(a["y"].as_f64().unwrap(), 134.0);
                if let Some(b) = cards.get(1) {
                    assert_eq!(b["y"].as_f64().unwrap(), a["y"].as_f64().unwrap() + a["h"].as_f64().unwrap() + 16.0, "the second under the first at its real height");
                    assert!(b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() <= 810.0, "the column ends above the bottom quarter: {b}");
                }
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn a_score_card_has_the_two_sides_and_the_clock_and_reads_clean() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit, mut s) = rig(Mode::Dark, scale);
            let mut o = Overlay::new(&json!({}));
            let score = json!({"league": "Premier League", "status": "78'", "live": true, "home": {"name": "Arsenal", "abbr": "ARS", "score": 2}, "away": {"name": "Chelsea", "abbr": "CHE", "score": 1}});
            o.posted(&[note("s", "score", "Arsenal 2, Chelsea 1", "", json!([]), score)], T0);
            o.refresh(134.0, &mut |e| f64::from(CardsSurface::height_of(&mut kit, e, 560.0, false)), T0);
            assert!(s.apply(&mut kit, o.shown(), &col(None), 1000).unwrap());
            assert_eq!(o.shown()[0].cls, "score");
            let found = lint_of(&mut kit, &s);
            assert!(found.is_empty(), "at {scale}: {found:?}");
        }
    }

    #[test]
    fn the_cards_go_under_the_camera_window_where_the_two_meet_and_a_card_that_has_gone_slides_out_and_is_dropped() {
        let (_h, mut kit, mut s) = rig(Mode::Dark, 1.0);
        let shown = two_cards(&mut kit);
        let popup = Size::new(560, 469);
        s.apply(&mut kit, &shown[..1], &col(Some(popup)), 1000).unwrap();
        let g = s.geometry();
        assert_eq!(g[0]["y"].as_f64().unwrap(), 134.0 + 469.0 + 16.0, "under the window: {g}");
        assert_eq!(s.top(Some(popup), 560.0), 134.0 + 469.0 + 16.0);
        // the card goes: it fades out and is dropped after the beat
        s.apply(&mut kit, &[], &col(None), 2000).unwrap();
        assert!(!s.is_up());
        assert!(s.due_at().is_some());
        let layers = s.layers();
        assert!(layers > 0, "still fading");
        assert!(s.tick(10_000));
        assert_eq!(s.layers(), 0, "nothing hidden is kept");
    }

    #[test]
    fn the_hint_says_what_ok_does_in_glyphs_with_a_word_only_where_the_record_gives_one() {
        let e = |actions: Value| cards::from_note(&note("n", "alert", "Low battery", "5 %", actions, Value::Null), &cards::Config::from_theme(&json!({})), T0).unwrap();
        let with = hints_of(&e(json!([{"id": "c", "label": "Clear", "action": "clear"}])), true);
        assert_eq!(with.iter().map(Hint::name).collect::<Vec<_>>(), vec!["OK, Clear".to_string(), "Back, close".to_string()]);
        let drawer = hints_of(&e(json!([])), true);
        assert_eq!(drawer.iter().map(Hint::name).collect::<Vec<_>>(), vec!["OK, open".to_string(), "Back, close".to_string()]);
        let none = hints_of(&e(json!([])), false);
        assert_eq!(none.iter().map(Hint::name).collect::<Vec<_>>(), vec!["OK, close".to_string()]);
    }
}
