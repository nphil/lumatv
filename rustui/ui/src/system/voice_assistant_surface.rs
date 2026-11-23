//! The voice assistant over everything: the glow round the edges ([`super::voice_glow`]), the words as they are heard, wiped in word by
//! word on a plate of the theme's glass in the lower part of the screen, and Home Assistant's answer on a card of the same glass that
//! takes their place and fades. It draws what the model hands it and takes no key, no pointer and no focus.
//!
//! Panel care: everything here is transient. The glow never rests (it flows, or breathes with reduced motion), the plate and the card
//! come with the turn and fade after the model's dwell (at most six seconds and a fade of 400 ms), the longest a turn stays is 35 s, and
//! the glass and the type are the theme's own, so nothing is brighter than the rest of the interface.

use super::voice_assistant::{CARD_FADE_MS, Card as Reply, Glow as Mode, Heard, Show, View};
use super::voice_glow::{Glow, Pace};
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Kit, icon, wrap};
use compositor_protocol::{BackdropId, Ease, Edges, Props, Rect, Rgba};
use lumatv_placement::{Rect as PlacedRect, above_subtitles, model::STACK, stack_beside};
use lumatv_tokens::TypeRole;
use std::io;
use ui::components::{Align, Style, text::line_height};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, Layer};

const MAX_W: f32 = 1280.0;
const MIN_W: f32 = 480.0;
/// The TV-safe bottom edge, where the toasts rest.
const FOOT: i32 = 1026;
const MAX_LINES: usize = 3;
const ICON: u32 = 40;
/// How soft the edge of the wipe is, in px.
const SOFT: f32 = 14.0;
/// The biggest type first: the words are as large as they can be in three lines.
const LADDER: [TypeRole; 3] = [TypeRole::Display, TypeRole::Title, TypeRole::Body];

/// The look and what is under it when a piece was made (generation, text size, Home's wallpaper blur, over an app): another makes it again.
type Made = (u64, u32, Option<BackdropId>, bool);

fn made(kit: &Kit, over_app: bool) -> Made {
    (kit.generation(), kit.text_scale.to_bits(), kit.wall_backdrop, over_app)
}

fn hidden() -> Props {
    Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) }
}

fn fade_out(layer: &Layer, from: Props, ms: u32) {
    let gone = Props { opacity: 0.0, visible: false, ..from };
    if ms == 0 {
        layer.set(gone);
    } else {
        layer.animate(gone, ms, Ease::In);
    }
}

/// A piece's layer at its resting place: fading in as it arrives, gliding when it moves, else at once.
fn place(kit: &Kit, arrive: bool, glide: bool, layer: &Layer, at: Props) {
    if arrive {
        kit.arrive(layer, at, Beat::Layer, false);
    } else if glide {
        kit.glide(layer, at, Beat::Layer);
    } else {
        layer.set(at);
    }
}

/// The text set in the biggest type of the ladder, from rung `first`, that takes at most three lines in `width` without cutting a word;
/// the smallest, with its last line elided, when none does.
struct Setting {
    rung: usize,
    style: Style,
    line: f32,
    rows: Vec<String>,
}

fn set_in(kit: &mut Kit, text: &str, width: f32, first: usize, ink: Rgba) -> Setting {
    let words = |s: &str| s.split_whitespace().count();
    let mut set = None;
    for (rung, role) in LADDER.iter().enumerate().skip(first.min(LADDER.len() - 1)) {
        let style = role_style(kit, *role, None, ink, Align::Left);
        let rows = wrap::lines(kit, style, text, width, MAX_LINES);
        let clean = rows.iter().map(|r| words(r)).sum::<usize>() == words(text);
        set = Some(Setting { rung, style, line: line_height(style.ty).round(), rows });
        if clean {
            break;
        }
    }
    set.expect("the ladder has a rung from `first` on")
}

/// A line of the words, wiped in from its left edge as they are heard.
struct Row {
    text: Text,
    /// The width of the first `k` words in `ends[k]`, the whole row's in the last.
    ends: Vec<f32>,
    w: f32,
    /// The words of the text before this row, and how many of its own are on.
    first: usize,
    on: usize,
}

impl Row {
    fn words(&self) -> usize {
        self.ends.len() - 1
    }

    /// The row at (`x`, `y`) with its first `k` words showing and the edge of the wipe soft; nothing of it for none.
    fn props(&self, at: (f32, f32), line: f32, k: usize) -> Props {
        if k == 0 {
            return Props { opacity: 0.0, visible: false, ..Props::at(at.0, at.1) };
        }
        self.wipe(at, line, self.ends[k.min(self.words())])
    }

    /// The row shown as far as `right` px from its left edge. The clip is in screen space, and a little taller than the type.
    fn wipe(&self, (x, y): (f32, f32), line: f32, right: f32) -> Props {
        Props {
            clip: Some(Rect { x: x - 2.0, y: y - 4.0, w: right + SOFT + 2.0, h: line + 8.0 }),
            feather: Edges::new(0.0, SOFT, 0.0, 0.0),
            ..Props::at(x, y)
        }
    }
}

/// What is heard, on a plate of the theme's glass.
struct Plate {
    pane: Pane,
    /// The device that heard it, when it was not the TV, and its width.
    label: Option<(Text, f32)>,
    rows: Vec<Row>,
    line: f32,
    pad: f32,
    /// Where the rows start below the plate's top.
    top: f32,
    turn: u64,
    epoch: u64,
    text: String,
    label_text: Option<String>,
    rung: usize,
    /// How many words are on.
    on: usize,
    made: Made,
    at: (f32, f32),
    size: (f32, f32),
    visible: bool,
    /// When a plate that has faded is let go.
    gone_at: Option<u64>,
}

impl Plate {
    fn row_at(&self, i: usize) -> (f32, f32) {
        (self.at.0 + ((self.size.0 - self.rows[i].w) / 2.0).round(), self.at.1 + self.top + i as f32 * self.line)
    }

    /// Each layer with where it rests; `full` has every word on.
    fn each(&self, full: bool, f: &mut dyn FnMut(&Layer, Props)) {
        f(self.pane.layer(), Props::at(self.at.0, self.at.1));
        if let Some((t, w)) = &self.label {
            f(t.layer(), Props::at(self.at.0 + ((self.size.0 - w) / 2.0).round(), self.at.1 + self.pad));
        }
        for (i, row) in self.rows.iter().enumerate() {
            f(row.text.layer(), row.props(self.row_at(i), self.line, if full { row.words() } else { row.on }));
        }
    }

    /// Wipes in the words up to `shown`, each over the time to the next.
    fn reveal(&mut self, kit: &Kit, shown: usize, step_ms: u32) {
        let added = shown.saturating_sub(self.on).max(1) as u32;
        let ms = if kit.ui.look.reduced_motion { 0 } else { step_ms.saturating_mul(added) };
        for i in 0..self.rows.len() {
            let (at, line) = (self.row_at(i), self.line);
            let row = &mut self.rows[i];
            let k = shown.saturating_sub(row.first).min(row.words());
            if k == row.on {
                continue;
            }
            let to = row.props(at, line, k);
            let layer = row.text.layer();
            if ms == 0 || k < row.on {
                layer.set(to);
            } else {
                if row.on == 0 {
                    layer.set(row.wipe(at, line, 0.0));
                }
                layer.animate(to, ms, Ease::Linear);
            }
            row.on = k;
        }
        self.on = shown;
    }
}

/// Home Assistant's answer, or why the turn failed, on a card of the same glass.
struct Answer {
    pane: Pane,
    icon: Option<ImageLayer>,
    caption: Option<Text>,
    rows: Vec<Text>,
    line: f32,
    pad: (f32, f32),
    /// Where the words start from the card's corner: right of the icon, below the caption.
    words_at: (f32, f32),
    turn: u64,
    text: String,
    ok: bool,
    caption_src: Option<String>,
    made: Made,
    at: (f32, f32),
    size: (f32, f32),
    visible: bool,
    gone_at: Option<u64>,
}

impl Answer {
    fn each(&self, f: &mut dyn FnMut(&Layer, Props)) {
        let (x, y) = self.at;
        f(self.pane.layer(), Props::at(x, y));
        if let Some(icon) = &self.icon {
            f(icon, Props::at(x + self.pad.0, y + self.pad.1));
        }
        if let Some(caption) = &self.caption {
            f(caption.layer(), Props::at(x + self.words_at.0, y + self.pad.1));
        }
        for (i, row) in self.rows.iter().enumerate() {
            f(row.layer(), Props::at(x + self.words_at.0, y + self.words_at.1 + i as f32 * self.line));
        }
    }
}

/// What the card says above the answer: what was heard, and which device heard it when it was not the TV.
fn caption_of(show: &Show) -> Option<String> {
    let heard = show.heard.as_ref().map(|h| h.text.as_str()).filter(|t| !t.is_empty());
    match (show.label.as_deref(), heard) {
        (Some(label), Some(heard)) => Some(format!("{label} \u{b7} {heard}")),
        (Some(only), None) | (None, Some(only)) => Some(only.to_string()),
        (None, None) => None,
    }
}

/// Where a piece of `size` rests: centred, its foot on the safe edge, above the film's subtitles when an app is in front, and above what
/// else is up (the volume bar, a toast, the format badge).
fn spot(size: (f32, f32), over_app: bool, avoid: &[PlacedRect]) -> (f32, f32) {
    let (w, h) = (size.0.round() as i32, size.1.round() as i32);
    let mut r = PlacedRect::new((1920 - w) / 2, FOOT - h, w, h);
    if over_app {
        r = above_subtitles(r, STACK);
    }
    for _ in 0..=avoid.len() {
        let before = r;
        for a in avoid {
            r = stack_beside(Some(a), r, STACK, false);
        }
        if r == before {
            break;
        }
    }
    (r.x as f32, r.y as f32)
}

fn build_plate(kit: &mut Kit, z: i32, show: &Show, heard: &Heard, rung: usize, on: usize, over_app: bool) -> io::Result<Plate> {
    let (pad_x, pad_y) = (kit.space(7), kit.space(6));
    let ink = kit.ui.look.ink();
    let set = set_in(kit, &heard.text, MAX_W - 2.0 * pad_x, rung, ink);
    let dim = kit.ui.look.colour(PaletteRole::Ink2);
    let cap = role_style(kit, TypeRole::Caption, None, dim, Align::Left);
    let cap_line = line_height(cap.ty).round();
    let mut rows = Vec::with_capacity(set.rows.len());
    let mut first = 0;
    for s in &set.rows {
        let words: Vec<&str> = s.split_whitespace().collect();
        let w = (width_of(kit, set.style, s) + 2.0).ceil();
        let mut ends = vec![0.0];
        for k in 1..=words.len() {
            ends.push(width_of(kit, set.style, &words[..k].join(" ")));
        }
        *ends.last_mut().expect("a row has its start") = w;
        let mut text = Text::new(kit, Frame::new(0.0, 0.0, w, set.line), z + 2, set.style, s)?;
        text.set_props(hidden());
        let done = on.saturating_sub(first).min(words.len());
        rows.push(Row { text, ends, w, first, on: done });
        first += words.len();
    }
    let room = MAX_W - 2.0 * pad_x;
    let label = match show.label.as_deref() {
        Some(l) => {
            let l = wrap::elide(kit, cap, l, room);
            let w = (width_of(kit, cap, &l) + 2.0).ceil();
            let mut t = Text::new(kit, Frame::new(0.0, 0.0, w, cap_line), z + 2, cap, &l)?;
            t.set_props(hidden());
            Some((t, w))
        }
        None => None,
    };
    let block = if label.is_some() { cap_line + kit.space(2) } else { 0.0 };
    let widest = rows.iter().map(|r| r.w).fold(label.as_ref().map_or(0.0, |l| l.1), f32::max);
    let size = ((widest + 2.0 * pad_x).clamp(MIN_W, MAX_W).ceil(), (2.0 * pad_y + block + set.line * rows.len() as f32).ceil());
    let pane = Pane::themed(kit, Frame::new(0.0, 0.0, size.0, size.1), z + 1, PaneKind::Card, over_app);
    pane.layer().set(hidden());
    Ok(Plate {
        pane,
        label,
        rows,
        line: set.line,
        pad: pad_y,
        top: pad_y + block,
        turn: show.turn,
        epoch: heard.epoch,
        text: heard.text.clone(),
        label_text: show.label.clone(),
        rung: set.rung,
        on,
        made: made(kit, over_app),
        at: (0.0, 0.0),
        size,
        visible: false,
        gone_at: None,
    })
}

fn build_answer(kit: &mut Kit, z: i32, show: &Show, reply: &Reply, over_app: bool) -> io::Result<Answer> {
    let (pad_x, pad_y, gap) = (kit.space(7), kit.space(6), kit.space(5));
    let col = pad_x + ICON as f32 + gap;
    let room = MAX_W - col - pad_x;
    let ink = kit.ui.look.ink();
    let set = set_in(kit, &reply.text, room, 1, ink);
    let dim = kit.ui.look.colour(PaletteRole::Ink2);
    let cap = role_style(kit, TypeRole::Caption, None, dim, Align::Left);
    let cap_line = line_height(cap.ty).round();
    let caption_src = caption_of(show);
    let caption_text = caption_src.as_ref().map(|c| wrap::elide(kit, cap, c, room));
    let block = if caption_text.is_some() { cap_line + kit.space(2) } else { 0.0 };
    let mut widest = 0.0f32;
    let mut rows = Vec::with_capacity(set.rows.len());
    for s in &set.rows {
        let w = (width_of(kit, set.style, s) + 2.0).ceil();
        widest = widest.max(w);
        let mut t = Text::new(kit, Frame::new(0.0, 0.0, w, set.line), z + 1, set.style, s)?;
        t.set_props(hidden());
        rows.push(t);
    }
    let caption = match &caption_text {
        Some(c) => {
            let w = (width_of(kit, cap, c) + 2.0).ceil();
            widest = widest.max(w);
            let mut t = Text::new(kit, Frame::new(0.0, 0.0, w, cap_line), z + 1, cap, c)?;
            t.set_props(hidden());
            Some(t)
        }
        None => None,
    };
    let tone = kit.ui.look.colour(if reply.ok { PaletteRole::Positive } else { PaletteRole::Danger });
    let icon = icon::layer(kit, if reply.ok { "check" } else { "triangle-alert" }, ICON, tone, z + 1, hidden())?;
    let size = ((col + widest + pad_x).clamp(MIN_W, MAX_W).ceil(), (2.0 * pad_y + block + set.line * rows.len() as f32).max(2.0 * pad_y + ICON as f32).ceil());
    let pane = Pane::themed(kit, Frame::new(0.0, 0.0, size.0, size.1), z, PaneKind::Card, over_app);
    pane.layer().set(hidden());
    Ok(Answer {
        pane,
        icon,
        caption,
        rows,
        line: set.line,
        pad: (pad_x, pad_y),
        words_at: (col, pad_y + block),
        turn: show.turn,
        text: reply.text.clone(),
        ok: reply.ok,
        caption_src,
        made: made(kit, over_app),
        at: (0.0, 0.0),
        size,
        visible: false,
        gone_at: None,
    })
}

pub struct VoiceSurface {
    z: i32,
    /// The glow and the look's generation it was baked for.
    glow: Option<(u64, Glow)>,
    plate: Option<Plate>,
    answer: Option<Answer>,
    /// The glass changed under the look: what is up is made again in place.
    restyled: bool,
}

impl VoiceSurface {
    /// `z`: the glow's; the plate and the card are above it.
    pub fn new(z: i32) -> Self {
        Self { z, glow: None, plate: None, answer: None, restyled: false }
    }

    pub fn restyle(&mut self) {
        self.restyled = true;
    }

    /// Draws the turn: the glow, the words, the card. True while anything is on the screen, a fade out included. `avoid` is what else is up
    /// at the foot of the screen; the plate and the card rest above it.
    pub fn apply(&mut self, kit: &mut Kit, view: &View, now: u64, over_app: bool, avoid: &[PlacedRect]) -> io::Result<bool> {
        if self.plate.as_ref().is_some_and(|p| !p.visible && p.gone_at.is_some_and(|at| now >= at)) {
            self.plate = None;
        }
        if self.answer.as_ref().is_some_and(|a| !a.visible && a.gone_at.is_some_and(|at| now >= at)) {
            self.answer = None;
        }
        let pace = view.as_ref().and_then(|s| match s.glow {
            Mode::Off => None,
            Mode::Listening => Some(Pace::Listening),
            Mode::Thinking => Some(Pace::Thinking),
        });
        self.light(kit, pace, now)?;
        let live = view.as_ref().filter(|s| !s.leaving);
        let words = live.and_then(|s| s.heard.as_ref().filter(|h| h.shown > 0 && s.card.is_none()).map(|h| (s, h)));
        let reply = live.and_then(|s| s.card.as_ref().map(|c| (s, c)));
        let fade = match view {
            Some(s) if s.leaving => kit.ui.look.duration_ms(CARD_FADE_MS as u32),
            _ => kit.beat_ms(Beat::Layer),
        };
        match words {
            Some((show, heard)) => self.show_plate(kit, show, heard, over_app, avoid)?,
            None => self.drop_plate(now, fade),
        }
        match reply {
            Some((show, reply)) => self.show_answer(kit, show, reply, over_app, avoid)?,
            None => self.drop_answer(now, fade),
        }
        self.restyled = false;
        Ok(self.plate.as_ref().is_some_and(|p| p.visible) || self.answer.as_ref().is_some_and(|a| a.visible) || self.glow.is_some())
    }

    /// The glow follows the turn; it is made when the first turn wants it and let go once it has faded, so that a plugin that is not
    /// used holds nothing.
    fn light(&mut self, kit: &mut Kit, pace: Option<Pace>, now: u64) -> io::Result<()> {
        if pace.is_some() && self.glow.as_ref().is_none_or(|(generation, _)| *generation != kit.generation()) {
            self.glow = Some((kit.generation(), Glow::new(kit, self.z)?));
        }
        if let Some((_, glow)) = self.glow.as_mut() {
            glow.set(kit, pace, now)?;
            glow.tick(now)?;
            if pace.is_none() && !glow.visible() {
                self.glow = None;
            }
        }
        Ok(())
    }

    fn show_plate(&mut self, kit: &mut Kit, show: &Show, heard: &Heard, over_app: bool, avoid: &[PlacedRect]) -> io::Result<()> {
        let key = made(kit, over_app);
        let continuing = self.plate.as_ref().is_some_and(|p| p.visible && p.turn == show.turn);
        let stale = self.restyled
            || self.plate.as_ref().is_none_or(|p| !p.visible || p.turn != show.turn || p.epoch != heard.epoch || p.text != heard.text || p.label_text != show.label || p.made != key);
        if stale {
            // words that grew carry on from where they were; other words, a new reveal
            let (rung, on) = self.plate.as_ref().filter(|p| continuing && p.epoch == heard.epoch).map_or((0, 0), |p| (p.rung, p.on));
            self.plate = Some(build_plate(kit, self.z, show, heard, rung, on, over_app)?);
        }
        let plate = self.plate.as_mut().expect("built above");
        let (x, y) = spot(plate.size, over_app, avoid);
        if !plate.visible || plate.at != (x, y) {
            let (arrive, glide) = (!plate.visible && !continuing, plate.visible);
            plate.at = (x, y);
            plate.each(false, &mut |layer, at| place(kit, arrive, glide, layer, at));
            plate.pane.move_to(x, y);
            plate.visible = true;
        }
        if heard.shown != plate.on {
            plate.reveal(kit, heard.shown, heard.step_ms);
        }
        Ok(())
    }

    fn show_answer(&mut self, kit: &mut Kit, show: &Show, reply: &Reply, over_app: bool, avoid: &[PlacedRect]) -> io::Result<()> {
        let key = made(kit, over_app);
        let continuing = self.answer.as_ref().is_some_and(|a| a.visible && a.turn == show.turn);
        let caption = caption_of(show);
        let stale = self.restyled
            || self.answer.as_ref().is_none_or(|a| !a.visible || a.turn != show.turn || a.text != reply.text || a.ok != reply.ok || a.made != key || a.caption_src != caption);
        if stale {
            self.answer = Some(build_answer(kit, self.z + 3, show, reply, over_app)?);
        }
        let answer = self.answer.as_mut().expect("built above");
        let (x, y) = spot(answer.size, over_app, avoid);
        if !answer.visible || answer.at != (x, y) {
            let (arrive, glide) = (!answer.visible && !continuing, answer.visible);
            answer.at = (x, y);
            answer.each(&mut |layer, at| place(kit, arrive, glide, layer, at));
            answer.pane.move_to(x, y);
            answer.visible = true;
        }
        Ok(())
    }

    fn drop_plate(&mut self, now: u64, ms: u32) {
        if let Some(p) = self.plate.as_mut().filter(|p| p.visible) {
            p.each(true, &mut |layer, at| fade_out(layer, at, ms));
            p.visible = false;
            p.gone_at = Some(now + u64::from(ms) + 60);
        }
    }

    fn drop_answer(&mut self, now: u64, ms: u32) {
        if let Some(a) = self.answer.as_mut().filter(|a| a.visible) {
            a.each(&mut |layer, at| fade_out(layer, at, ms));
            a.visible = false;
            a.gone_at = Some(now + u64::from(ms) + 60);
        }
    }

    /// The plate and the card that are up.
    pub fn rects(&self) -> Vec<Rect> {
        let plate = self.plate.as_ref().filter(|p| p.visible).map(|p| Rect { x: p.at.0, y: p.at.1, w: p.size.0, h: p.size.1 });
        let answer = self.answer.as_ref().filter(|a| a.visible).map(|a| Rect { x: a.at.0, y: a.at.1, w: a.size.0, h: a.size.1 });
        plate.into_iter().chain(answer).collect()
    }

    /// When the surface wants a tick of its own: the glow's flow and breath, and letting go of what has faded.
    pub fn due_at(&self) -> Option<u64> {
        [self.glow.as_ref().and_then(|(_, g)| g.due_at()), self.plate.as_ref().and_then(|p| p.gone_at), self.answer.as_ref().and_then(|a| a.gone_at)].into_iter().flatten().min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use lumatv_tokens::Tokens;
    use serde_json::json;
    use ui::lucent::palette::Mode as Look;

    fn heard(text: &str, epoch: u64, shown: usize) -> Heard {
        Heard { text: text.into(), epoch, shown, step_ms: 75 }
    }

    fn show(glow: Mode, heard: Option<Heard>, card: Option<(&str, bool)>) -> View {
        Some(Show { turn: 1, glow, heard, card: card.map(|(t, ok)| Reply { text: t.into(), ok }), label: None, leaving: false })
    }

    const SAID: &str = "turn on the kitchen lights and then dim the living room lamp to about a third";

    #[test]
    fn the_busiest_state_is_a_glow_a_plate_and_a_card_and_fits_the_budget() {
        let (_h, mut kit) = testkit::kit(1.0);
        let before = kit.room();
        let mut surface = VoiceSurface::new(1000);
        let all = SAID.split_whitespace().count();
        surface.apply(&mut kit, &show(Mode::Thinking, Some(heard(SAID, 1, all)), None), 0, false, &[]).unwrap();
        let reply = "Okay, I have turned on the kitchen lights and set the living room lamp to thirty three percent for you.";
        let mut view = show(Mode::Thinking, Some(heard(SAID, 1, all)), Some((reply, true)));
        view.as_mut().unwrap().label = Some("Living Room Echo Dot".into());
        surface.apply(&mut kit, &view, 100, false, &[]).unwrap();
        let after = kit.room();
        let used = (before.layers - after.layers, before.groups - after.groups, before.tracks - after.tracks);
        assert!(used.0 <= 40 && used.1 <= 12 && used.2 <= 12, "{used:?}");
        assert!(surface.plate.is_some() && surface.answer.is_some(), "both are up while one fades into the other");
    }

    #[test]
    fn words_that_grow_are_wiped_in_and_other_words_start_again() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut surface = VoiceSurface::new(1000);
        surface.apply(&mut kit, &show(Mode::Listening, Some(heard(SAID, 1, 2)), None), 0, false, &[]).unwrap();
        let plate = surface.plate.as_ref().unwrap();
        let first = plate.rows[0].text.layer().id();
        assert_eq!(plate.rows.iter().map(|r| r.on).sum::<usize>(), 2);
        surface.apply(&mut kit, &show(Mode::Listening, Some(heard(SAID, 1, 5)), None), 100, false, &[]).unwrap();
        let plate = surface.plate.as_ref().unwrap();
        assert_eq!(plate.rows.iter().map(|r| r.on).sum::<usize>(), 5);
        assert!(plate.rows[0].text.layer().id() == first, "the same lines, more of them on");
        surface.apply(&mut kit, &show(Mode::Listening, Some(heard("what time is it", 2, 1)), None), 200, false, &[]).unwrap();
        let plate = surface.plate.as_ref().unwrap();
        assert!(plate.rows[0].text.layer().id() != first, "replaced words are laid out again");
        assert_eq!(plate.rows.iter().map(|r| r.on).sum::<usize>(), 1);
    }

    #[test]
    fn the_card_takes_the_words_place_and_leaving_takes_everything_away() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut surface = VoiceSurface::new(1000);
        assert!(!surface.apply(&mut kit, &None, 0, false, &[]).unwrap(), "nothing to draw is nothing held");
        assert!(surface.rects().is_empty());
        surface.apply(&mut kit, &show(Mode::Off, Some(heard("lights", 1, 1)), None), 0, false, &[]).unwrap();
        assert_eq!(surface.rects().len(), 1);
        let mut view = show(Mode::Off, Some(heard("lights", 1, 1)), Some(("Done", true)));
        assert!(surface.apply(&mut kit, &view, 100, false, &[]).unwrap());
        assert_eq!(surface.rects().len(), 1, "the card, not the plate");
        view.as_mut().unwrap().leaving = true;
        assert!(!surface.apply(&mut kit, &view, 200, false, &[]).unwrap());
        assert!(surface.rects().is_empty());
        for _ in 0..2 {
            let at = surface.due_at().unwrap();
            surface.apply(&mut kit, &None, at, false, &[]).unwrap();
        }
        assert!(surface.plate.is_none() && surface.answer.is_none() && surface.due_at().is_none(), "what has faded is let go");
    }

    #[test]
    fn the_plate_and_the_card_rest_centred_inside_the_safe_edge_above_what_else_is_up() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            let mut surface = VoiceSurface::new(1000);
            surface.apply(&mut kit, &show(Mode::Off, Some(heard(SAID, 1, 6)), None), 0, false, &[]).unwrap();
            let r = surface.rects()[0];
            assert!(r.w <= MAX_W && r.w >= MIN_W && r.y >= 0.0);
            assert_eq!(r.y + r.h, FOOT as f32);
            assert!((r.x + r.w / 2.0 - 960.0).abs() <= 1.0);
            let toast = PlacedRect::new(740, 940, 440, 86);
            surface.apply(&mut kit, &show(Mode::Off, Some(heard(SAID, 1, 6)), None), 50, false, &[toast]).unwrap();
            assert_eq!(surface.rects()[0].y + surface.rects()[0].h, 940.0 - STACK as f32);
            surface.apply(&mut kit, &show(Mode::Off, None, Some(("Done", false))), 100, true, &[]).unwrap();
            let card = surface.rects().pop().unwrap();
            assert!(card.y + card.h == 889.0 && card.w >= MIN_W);
        }
    }

    #[test]
    fn a_voice_satellite_is_named_on_the_plate_and_the_card_and_the_tv_alone_shows_no_name() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut surface = VoiceSurface::new(1000);
        let mut view = show(Mode::Off, Some(heard("lights", 1, 1)), None);
        surface.apply(&mut kit, &view, 0, false, &[]).unwrap();
        assert!(surface.plate.as_ref().unwrap().label.is_none());
        view.as_mut().unwrap().label = Some("Living Room Echo Dot".into());
        surface.apply(&mut kit, &view, 50, false, &[]).unwrap();
        assert_eq!(surface.plate.as_ref().unwrap().label.as_ref().map(|l| l.0.text().to_string()).as_deref(), Some("Living Room Echo Dot"));
        view.as_mut().unwrap().card = Some(Reply { text: "Done".into(), ok: true });
        surface.apply(&mut kit, &view, 100, false, &[]).unwrap();
        assert_eq!(surface.answer.as_ref().unwrap().caption.as_ref().map(|c| c.text().to_string()).as_deref(), Some("Living Room Echo Dot \u{b7} lights"));
    }

    #[test]
    fn a_new_theme_makes_the_glass_and_the_glow_again_in_its_colours() {
        let (mut h, mut kit) = testkit::kit(1.0);
        let mut surface = VoiceSurface::new(1000);
        let view = show(Mode::Thinking, Some(heard("lights", 1, 1)), None);
        surface.apply(&mut kit, &view, 0, false, &[]).unwrap();
        let glass = |h: &mut ui::testing::Harness, surface: &VoiceSurface| {
            h.commit();
            let id = surface.plate.as_ref().unwrap().pane.layer().id();
            h.layers().into_iter().find(|l| l.0 == id).map(|l| l.1)
        };
        let dark = glass(&mut h, &surface);
        let generation = surface.glow.as_ref().unwrap().0;
        kit.retheme(Tokens::from_json(&json!({})), Look::Light, 1.0);
        surface.restyle();
        surface.apply(&mut kit, &view, 50, false, &[]).unwrap();
        assert_ne!(glass(&mut h, &surface), dark, "the glass follows the theme");
        assert_ne!(surface.glow.as_ref().unwrap().0, generation, "the glow is baked again");
        assert_eq!(surface.glow.as_ref().unwrap().0, kit.generation());
    }
}
