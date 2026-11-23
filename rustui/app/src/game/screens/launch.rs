//! The start screen of a launch and what the app draws over the running stream: the strip, the figures pill and the drawers.
//!
//! The start screen is a flat canvas: the game's logo, a turning ring and one line of what the launch is doing. The ring is
//! `RING_FRAMES` images made once, one per turn of the arc; a step only changes which one is opaque, so nothing is drawn again.
//! Over the stream the app draws only opaque sheets, never glass: nothing behind them is captured.
//!
//! Node ids: 700 the logo, 701 the line, 702 the caption under it, 703 the ring's track, `LIVE` + 300 and up the ring's images; the
//! figures pill takes two of the HUD's ids (it sits above y = 150, which the page's cull spares only for them); 8300 and up the rows of
//! a drawer, 20 to a row.
use super::{
    paint::{Fill, H, HUD, LIVE, Paint, W},
    parts::{
        SHEET_Z, Side,
        dialog::{self, Action},
        drawer, hints,
        motion::Enter,
        row::{self, Leading, Line},
        stage::logo_art,
        strip::{self, Item, Sign},
    },
    stats,
    symbols::Symbol,
};
use crate::game::model::{
    Focus, Layer,
    launch::{Achieved, RING_FRAMES, StripItem},
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Align, Icon, ellipsise},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

const IDENTITY: u64 = 700;
const HEADLINE: u64 = 701;
const CAPTION: u64 = 702;
const TRACK: u64 = 703;
const RING: u64 = LIVE.start + 300;
const PILL: u64 = HUD.start + 2;
const ROWS: u64 = 8300;

/// The logo's box, the ring and the room between the blocks of the stack.
const LOGO: (f32, f32) = (520.0, 150.0);
const RING_SIDE: f32 = 92.0;
const GAP: f32 = 30.0;
/// The room between the line and its caption.
const UNDER: f32 = 6.0;
/// The light a dimmed logo and line rest at.
const REST: f32 = 0.4;
/// The figures pill: room at the sides and above and below the figures, and between them.
const PILL_PAD_X: f32 = 26.0;
const PILL_PAD_Y: f32 = 12.0;
const PILL_GAP: f32 = 22.0;
/// A drawer's rows, and how many an achievements list shows at once.
const ROW_H: f32 = 88.0;
const ROW_GAP: f32 = 8.0;
const WINDOW: usize = 7;

/// Draws one line of text centred on the screen's middle, at most `width` wide: a title line in bold, anything else as a caption.
/// Returns the height it took.
fn centred(p: &mut Paint<'_>, id: u64, text: &str, role: Type, y: f32, width: f32) -> f32 {
    let (ink, bold) = if matches!(role, Type::Title) { (Role::Ink, true) } else { (Role::Ink2, false) };
    let ty = if bold { p.bold(role) } else { p.ty(role) };
    let shown = ellipsise(p.fonts, text, ty, width);
    let mut word = p.word(shown, role, p.ink(ink, false), 0.0, 0.0, width);
    word.ty = ty;
    word.align = Align::Centre;
    p.words(id, Frame::new((W - width) / 2.0, y, width, p.line(role)), vec![word], vec![], 12);
    p.plan.nodes.last().map_or(0.0, |node| node.frame.h)
}
/// Sets the light of every node from `from` on.
fn light(p: &mut Paint<'_>, from: usize, opacity: f32) {
    for node in &mut p.plan.nodes[from..] {
        node.props.opacity = opacity;
    }
}

/// The start screen: the logo, the ring and the step the launch is at. Both the logo and the line dim after a minute without a key;
/// the ring keeps turning, so the screen still reads as alive.
pub fn launch(p: &mut Paint<'_>) {
    let m = p.model;
    let (Some(game), Some(step)) = (m.session().map(|s| s.game), m.start_step()) else { return };
    let reduced = m.prefs.reduce_motion || p.tokens.motion.reduced;
    let width = (W - 2.0 * p.x).min(1200.0);
    let head_h = p.line(Type::Title);
    let caption_h = step.caption.as_ref().map_or(0.0, |_| UNDER + p.line(Type::Body));
    let mut y = (H - (LOGO.1 + GAP + RING_SIDE + GAP + head_h + caption_h)) / 2.0;
    let dim = m.start_dim();

    let from = p.plan.nodes.len();
    let logo = Frame::new((W - LOGO.0) / 2.0, y, LOGO.0, LOGO.1);
    if let Some(path) = logo_art(p, game) {
        p.image(IDENTITY, path, logo, 0.0, true, None, 10);
    } else if let Some(title) = m.game(game).map(|g| g.title.clone()) {
        let lines = p.fit(&title, Type::Title, logo.w, p.rows(Type::Title, logo.h));
        let used = p.block(Type::Title, lines.len());
        let top = logo.y + (logo.h - used) / 2.0;
        for (i, line) in lines.iter().enumerate() {
            centred(p, IDENTITY + 100 + i as u64, line, Type::Title, top + i as f32 * p.line(Type::Title), logo.w);
        }
    }
    y += LOGO.1 + GAP;
    let ring = Frame::new((W - RING_SIDE) / 2.0, y, RING_SIDE, RING_SIDE);
    y += RING_SIDE + GAP;
    y += centred(p, HEADLINE, &step.headline, Type::Title, y, width);
    if let Some(caption) = &step.caption {
        centred(p, CAPTION, caption, Type::Body, y + UNDER, width);
    }
    if dim {
        light(p, from, REST);
    }

    // The track stands still. Each image of the arc is a fixed symbol drawn once; only one is opaque at a time. Reduced motion
    // has one image that does not move, and dims with the words.
    let ring_from = p.plan.nodes.len();
    p.symbol(TRACK, Symbol::Ring, ring, p.ink(Role::TrackOff, false), 10);
    let shown = m.ring_frame();
    for k in 0..RING_FRAMES {
        if reduced && k != 0 {
            continue;
        }
        p.symbol(RING + u64::from(k), Symbol::Spinner { turn: k, of: RING_FRAMES }, ring, p.ink(Role::Accent, false), 11);
        let at = p.plan.nodes.len() - 1;
        p.plan.nodes[at].props.opacity = if k == shown || reduced { 1.0 } else { 0.0 };
    }
    if reduced && dim {
        light(p, ring_from, REST);
    }
    let hints_now = m.hints();
    hints::bar(p, &hints_now, Side::Right, m.hints_dim);
}

/// The page while the stream runs: nothing, the strip with the figures, a drawer or the End prompt.
pub fn running(p: &mut Paint<'_>) {
    match p.model.view.top() {
        Some(Layer::Picture) => picture(p),
        Some(Layer::Achievements) => achievements(p),
        Some(Layer::EndConfirm) => end(p),
        _ => bare(p),
    }
}

fn icon(item: StripItem) -> Sign {
    match item {
        StripItem::Resume => Sign::Icon(Icon::Play),
        StripItem::Picture => Sign::Icon(Icon::Sun),
        StripItem::Controller => Sign::Symbol(Symbol::Gamepad),
        StripItem::Achievements => Sign::Symbol(Symbol::Trophy),
        StripItem::End => Sign::Icon(Icon::Power),
    }
}

/// The stats overlay when it is on, and for the first seconds and after each key the strip with the figures.
fn bare(p: &mut Paint<'_>) {
    stats::show(p);
    let m = p.model;
    if !m.strip_visible() {
        return;
    }
    pill(p);
    let items: Vec<Item<'_>> = StripItem::ALL
        .iter()
        .enumerate()
        .map(|(i, item)| Item { icon: icon(*item), label: Some(item.label()), focus: Focus::Strip(i), danger: *item == StripItem::End })
        .collect();
    let focused = match m.view.focus {
        Focus::Strip(i) => Some(i),
        _ => None,
    };
    strip::bar(p, &items, focused);
    let hints_now = m.hints();
    hints::bar(p, &hints_now, Side::Right, m.hints_dim);
}

/// The stream's figures, top right: the picture, the frame rate and the round trip. The stats overlay says the same in full, so with
/// it on there is no pill.
fn pill(p: &mut Paint<'_>) {
    let m = p.model;
    let figures = m.figures();
    if m.hud_visible() || figures.is_empty() {
        return;
    }
    let ty = p.bold(Type::Label);
    let colour = p.ink(Role::Ink, false);
    let mut words = Vec::new();
    let mut x = PILL_PAD_X;
    for (i, text) in figures.iter().enumerate() {
        if i > 0 {
            x += PILL_GAP;
        }
        let (mut cut, w) = p.figures(text, ty, colour, 0.0, PILL_PAD_Y);
        for word in &mut cut {
            word.x += x;
        }
        words.extend(cut);
        x += w;
    }
    let frame = Frame::new(W - p.x - (x + PILL_PAD_X), p.y, x + PILL_PAD_X, p.line(Type::Label) + 2.0 * PILL_PAD_Y);
    p.arrives(frame, Enter::FADE);
    p.surface(PILL, frame, Fill::Sheet, Radius::Pill, false, false, false, SHEET_Z);
    p.words(PILL + 1, frame, words, vec![], SHEET_Z + 1);
}

/// What a drawer's rows are drawn with sits under the drawer's sheet; this lifts it over.
fn above_sheet(p: &mut Paint<'_>, from: usize) {
    for node in &mut p.plan.nodes[from..] {
        node.z += SHEET_Z;
    }
}

fn picture(p: &mut Paint<'_>) {
    let m = p.model;
    let content = drawer::open(p, Side::Right, drawer::WIDTH, "Picture");
    let from = p.plan.nodes.len();
    let mut y = content.y;
    for (i, picture) in m.picture_rows().iter().enumerate() {
        let frame = Frame::new(content.x, y, content.w, ROW_H);
        let line = Line { lead: Leading::Glyph(picture.icon), name: picture.label, sub: None };
        let (id, focused) = (ROWS + 20 * i as u64, p.focused(Focus::SheetRow(i)));
        if picture.switch {
            row::switch(p, id, frame, &line, picture.on, focused);
        } else {
            row::stepper(p, id, frame, &line, &picture.value, focused);
        }
        p.hit(Focus::SheetRow(i), frame);
        y += ROW_H + ROW_GAP;
    }
    above_sheet(p, from);
    let hints_now = m.hints();
    hints::bar(p, &hints_now, Side::Left, m.hints_dim);
}

fn achievements(p: &mut Paint<'_>) {
    let m = p.model;
    let content = drawer::open(p, Side::Right, drawer::WIDTH, "Achievements");
    let from = p.plan.nodes.len();
    let inner = Frame::new(content.x + 28.0, content.y, content.w - 56.0, 0.0);
    match m.achievement_list() {
        Achieved::Words(text) => p.text(ROWS, &text, inner, Type::Body, Role::Ink2, false, 12),
        Achieved::List { summary, rows } => {
            p.text(ROWS, &summary, inner, Type::Body, Role::Ink2, false, 12);
            let mut y = content.y + p.line(Type::Body) + 20.0;
            let at = match m.view.focus {
                Focus::SheetRow(i) => i,
                _ => 0,
            };
            let first = (at + 1).saturating_sub(WINDOW).min(rows.len().saturating_sub(WINDOW));
            for (slot, (i, achievement)) in rows.iter().enumerate().skip(first).take(WINDOW).enumerate() {
                let frame = Frame::new(content.x, y, content.w, ROW_H);
                let lead = Leading::Glyph(if achievement.unlocked { Icon::Check } else { Icon::Circle });
                let line = Line { lead, name: &achievement.name, sub: Some(&achievement.note) };
                row::value(p, ROWS + 20 * (slot as u64 + 1), frame, &line, "", None, p.focused(Focus::SheetRow(i)));
                p.hit(Focus::SheetRow(i), frame);
                y += ROW_H + ROW_GAP;
            }
        }
    }
    above_sheet(p, from);
    let hints_now = m.hints();
    hints::bar(p, &hints_now, Side::Left, m.hints_dim);
}

/// The prompt before a session ends, on an opaque backdrop. Focus starts on Keep playing, which is also what a half minute of quiet
/// chooses. Nothing behind a sheet over the stream is captured, so the buttons rest as plain tint; the one that ends carries the danger
/// icon, as every dialog's does.
fn end(p: &mut Paint<'_>) {
    let m = p.model;
    let (title, body, confirm) = m.end_copy();
    let keep = Action { label: "Keep playing", icon: Some(Icon::Play), focus: Focus::SheetAction(0) };
    let leave = Action { label: confirm, icon: Some(Icon::Power), focus: Focus::SheetAction(1) };
    let from = p.plan.nodes.len();
    dialog::modal(p, title, &body, &[keep, leave], 0);
    dialog::opaque(p, from);
    dialog::danger(p, 1);
    let hints_now = m.hints();
    hints::bar(p, &hints_now, Side::Left, m.hints_dim);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        model::{
            Input, Key, Model,
            tests::{describe, key, launching, report},
        },
        screens::{
            paint::{Body, Node, Plan, assert_readable},
            plan_in,
        },
    };
    use lumagame_core::{engine::Feature, ipc::SessionState};
    use ui::lucent::palette::Mode;

    /// A title the logo's box holds only in part.
    const LONG: &str = "The Extraordinarily Long-Winded Chronicles of Sir Bartholomew Fitzwilliam-Montgomery the Third and the Quest for the Unpronounceable Artefact";

    fn queued(title: &str) -> Model {
        let mut m = launching(title, false);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame, Feature::Queue]);
        report(&mut m, SessionState::Queue { position: Some(12), eta_s: Some(185) });
        m
    }
    fn running() -> Model {
        let mut m = launching("Hades", false);
        let stream = serde_json::json!({"codec":"h265","width":3840,"height":2160,"fps":120,"hdr":true,"audio_channels":2});
        report(&mut m, SessionState::Running { stream: Some(serde_json::from_value(stream).unwrap()) });
        m.view.stats.rtt_ms = Some(18.0);
        m
    }
    fn node(plan: &Plan, id: u64) -> &Node {
        plan.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("node {id}"))
    }
    /// The ring's images, by turn, that are opaque.
    fn lit(plan: &Plan) -> Vec<u64> {
        plan.nodes.iter().filter(|n| LIVE.contains(&n.id) && n.id >= RING && n.props.opacity > 0.5).map(|n| n.id - RING).collect()
    }

    #[test]
    fn the_start_screen_keeps_its_blocks_apart_inside_the_screen_at_both_text_sizes_in_both_themes() {
        let mut fonts = ui::Fonts::new();
        for percent in [100, 200] {
            for mode in [Mode::Dark, Mode::Light] {
                for title in [LONG, "Hades"] {
                    let mut m = queued(title);
                    m.prefs.text_percent = percent;
                    let plan = plan_in(&m, mode);
                    assert_readable(&plan, &mut fonts, false);
                    let (ring, head, caption) = (node(&plan, TRACK).frame, node(&plan, HEADLINE).frame, node(&plan, CAPTION).frame);
                    for f in [ring, head, caption] {
                        assert!(f.x >= 0.0 && f.x + f.w <= W && f.y >= 0.0 && f.y + f.h <= H, "{percent}% {mode:?} {title:.8}: {f:?} leaves the screen");
                    }
                    assert!(head.y >= ring.y + ring.h && caption.y >= head.y + head.h, "{percent}% {mode:?}: the ring, the line and the caption stack in that order");
                }
            }
        }
    }
    #[test]
    fn the_ring_is_fixed_images_that_swap_by_opacity_and_one_still_image_when_motion_is_reduced() {
        let mut m = queued("Hades");
        let first = plan_in(&m, Mode::Dark);
        assert_eq!((lit(&first), first.nodes.iter().filter(|n| n.id >= RING && LIVE.contains(&n.id)).count()), (vec![0], usize::from(RING_FRAMES)));
        m.step(Input::Tick { now_ms: 225, unix_s: 0 });
        let later = plan_in(&m, Mode::Dark);
        assert_eq!(lit(&later), [3], "a step only changes which image is opaque");
        for k in 0..RING_FRAMES {
            let Body::Symbol(Symbol::Spinner { turn, of }, _) = &node(&later, RING + u64::from(k)).body else { panic!("image {k} is not a ring symbol") };
            assert_eq!((*turn, *of), (k, RING_FRAMES), "image {k} is the same symbol at every step, so it is drawn once");
        }
        m.prefs.reduce_motion = true;
        let still = plan_in(&m, Mode::Dark);
        assert_eq!((lit(&still), still.nodes.iter().filter(|n| n.id >= RING && LIVE.contains(&n.id)).count()), (vec![0], 1));
    }
    #[test]
    fn after_a_minute_the_logo_and_line_dim_and_the_ring_keeps_its_light() {
        let mut m = queued("Hades");
        m.step(Input::Tick { now_ms: 60_000, unix_s: 60 });
        let plan = plan_in(&m, Mode::Dark);
        assert_eq!([IDENTITY + 100, HEADLINE, CAPTION].map(|id| node(&plan, id).props.opacity), [REST; 3]);
        assert_eq!((node(&plan, TRACK).props.opacity, lit(&plan).len()), (1.0, 1), "the ring keeps turning at full light");
    }
    #[test]
    fn the_strip_has_its_five_items_and_the_figures_pill_and_leaves_with_a_key_back() {
        let mut m = running();
        let plan = plan_in(&m, Mode::Dark);
        let items: Vec<usize> = plan.hits.iter().filter_map(|(f, _)| if let Focus::Strip(i) = f { Some(*i) } else { None }).collect();
        assert_eq!(items, [0, 1, 2, 3, 4]);
        assert!(plan.nodes.iter().any(|n| n.id == PILL), "no pill: figures {:?}, hud {}, ids {:?}", m.figures(), m.hud_visible(), plan.nodes.iter().map(|n| n.id).collect::<Vec<_>>());
        let pill = node(&plan, PILL).frame;
        assert!(pill.x + pill.w <= W && pill.y >= 0.0 && pill.x > W / 2.0, "the pill sits top right: {pill:?}");
        key(&mut m, Key::Back);
        let bare = plan_in(&m, Mode::Dark);
        assert!(bare.hits.is_empty() && bare.nodes.iter().all(|n| n.id != PILL));
    }
    #[test]
    fn the_drawers_and_the_end_prompt_draw_their_rows_and_actions_over_the_stream() {
        let mut m = running();
        key(&mut m, Key::Right);
        key(&mut m, Key::Ok);
        let rows = |m: &Model| plan_in(m, Mode::Dark).hits.iter().filter(|(f, _)| matches!(f, Focus::SheetRow(_))).count();
        assert_eq!(rows(&m), 3, "Stream, HDR and Stats");
        key(&mut m, Key::Back);
        m.view.focus = Focus::Strip(4);
        key(&mut m, Key::Ok);
        let end = plan_in(&m, Mode::Dark);
        let actions: Vec<Focus> = end.hits.iter().map(|(f, _)| *f).filter(|f| matches!(f, Focus::SheetAction(_))).collect();
        assert_eq!(actions, [Focus::SheetAction(0), Focus::SheetAction(1)]);
    }
}
