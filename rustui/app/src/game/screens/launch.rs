//! The launch curtain: the game's art on the stage, three step chips, one status line and a bar that moves only as far as the wait
//! has. It is laid out from the bottom up on measured text, so no block can reach into the next at any text size.
use super::{
    pages::{logo_art, stage},
    paint::{Fill, H, Paint, Sweep, W},
};
use crate::game::model::Focus;
use compositor_protocol::{Edges, Props, Rect};
use lumagame_core::session::Phase;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Icon, line_height},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

const IDENTITY: u64 = 100;
const CHIPS: u64 = 120;
const STATUS: u64 = 140;
const DETAIL: u64 = 141;
const CANCEL: u64 = 150;
const TRACK: u64 = 160;
/// The bar's moving part: the shell eases its width at the scroll time rather than the focus time.
pub const FILL: u64 = 161;
/// The logo's art fits this frame, at its bottom left.
const LOGO: (f32, f32) = (640.0, 240.0);
/// How wide the text runs: the part of the art the curtain keeps dark, so what is written there reads over any game.
const COLUMN: f32 = 1100.0;
const BAR_W: f32 = 760.0;
/// Between the blocks, top down: identity to steps, steps to status, status to bar, bar to Cancel.
const GAPS: [f32; 4] = [44.0, 36.0, 28.0, 40.0];
/// Between the status and the wait written under it.
const UNDER: f32 = 10.0;

pub fn launch(p: &mut Paint<'_>) {
    let Some(session) = p.model.session() else {
        return;
    };
    let (game, phase) = (session.game, session.phase());
    stage(p, game, H, 0.0, H, false, true);
    let x = p.x;
    let col = COLUMN.min(W - 2.0 * x);
    let reduced = p.model.prefs.reduce_motion || p.tokens.motion.reduced;

    // The status takes the big size while it fits the column and the title size when it does not; the wait goes beside it when
    // both fit on one line, and under it when not.
    let (head, wait) = status(phase);
    let queued = matches!(phase, Phase::Queue { position: Some(_), .. });
    let role = if queued && p.width(&head, Type::Display) <= col { Type::Display } else { Type::Title };
    let head = p.fit(&head, role, col, 2);
    let head_w = head.iter().map(|line| p.width(line, role)).fold(0.0, f32::max);
    let gap = (p.ty(role).px as f32 * 0.4).round();
    let wait = wait.map(|text| {
        let beside = format!("· {text}");
        if head.len() == 1 && head_w + gap + p.width(&beside, Type::Title) <= col { (beside, true) } else { (text, false) }
    });
    let head_h = p.block(role, head.len());
    let status_h = head_h + wait.as_ref().map_or(0.0, |(_, beside)| if *beside { 0.0 } else { UNDER + p.block(Type::Title, 1) });

    // Bottom up: each block sits above the one under it, whatever height its text came to.
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    let cancel_y = H - p.y - button_h;
    let bar_h = (6.0 * p.scale).round();
    let bar = Rect { x, y: cancel_y - GAPS[3] - bar_h, w: BAR_W.min(col), h: bar_h };
    let status_y = bar.y - GAPS[2] - status_h;
    let chip_h = 52.0f32.max(p.line(Type::Label) + 12.0);
    let chips_y = status_y - GAPS[1] - chip_h;
    let art = logo_art(p, game);
    let title = if art.is_some() {
        Vec::new()
    } else {
        let name = p.model.game(game).map(|g| g.title.clone()).unwrap_or_default();
        p.fit(&name, Type::Title, col, 2)
    };
    let identity_h = if art.is_some() { LOGO.1 } else { p.block(Type::Title, title.len()) };
    let identity_y = chips_y - GAPS[0] - identity_h;

    match art {
        Some(path) => p.image(IDENTITY, path, Frame::new(x, identity_y, LOGO.0, LOGO.1), 0.0, true, None, 10),
        None => {
            p.lines(IDENTITY, title, Frame::new(x, identity_y, col, 0.0), Type::Title, Role::Ink, true, 10);
        }
    }
    let step = match phase {
        Phase::Checking | Phase::SignIn => 0,
        Phase::Queue { .. } => 1,
        _ => 2,
    };
    let mut chip_x = x;
    for (i, label) in ["Checking", "In queue", "Starting"].into_iter().enumerate() {
        let f = Frame::new(chip_x, chips_y, p.width(label, Type::Label) + 74.0, chip_h);
        let id = CHIPS + i as u64 * 3;
        p.surface(id, f, Fill::Tint(if step == i { 0.12 } else { 0.04 }), Radius::Pill, true, false, false, 10);
        p.words(
            id + 1,
            f,
            vec![p.word(label, Type::Label, p.ink(if step == i { Role::Ink } else { Role::Ink2 }, true), 42.0, (f.h - p.line(Type::Label)) / 2.0, f.w - 60.0)],
            vec![p.mark(
                if i < step { Icon::Tick } else { Icon::Circle },
                22.0,
                14.0,
                (f.h - 22.0) / 2.0,
                if i < step { Role::Positive } else { Role::Ink2 },
                true,
            )],
            11,
        );
        chip_x += f.w + 14.0;
    }
    if let Some((text, beside)) = wait {
        // Two sizes on one line share a baseline, not a top.
        let (dx, dy) = if beside { (head_w + gap, baseline(p, &head[0], role) - baseline(p, &text, Type::Title)) } else { (0.0, head_h + UNDER) };
        p.text(DETAIL, &text, Frame::new(x + dx, status_y + dy, col - dx, 0.0), Type::Title, Role::Ink2, true, 10);
    }
    p.lines(STATUS, head, Frame::new(x, status_y, col, 0.0), role, Role::Ink, true, 10);

    // A bar for the wait only where the wait can be measured, as far as it has gone. Otherwise a segment keeps crossing the track;
    // with reduced motion there is no bar, because a still, empty one reads as no progress.
    let track = Frame::new(bar.x, bar.y, bar.w, bar.h);
    let ink = p.ink(Role::Ink, true);
    match p.model.queue_progress() {
        Some(share) => {
            p.solid(TRACK, track, p.ink(Role::TrackOff, true), bar_h / 2.0, 12);
            p.solid(FILL, track, ink, bar_h / 2.0, 13);
            window(p, Rect { w: bar.w * share, ..bar }, Edges::NONE);
        }
        None if !reduced => {
            p.solid(TRACK, track, p.ink(Role::TrackOff, true), bar_h / 2.0, 12);
            let segment = (bar.w / 3.0).round();
            p.solid(FILL, Frame::new(bar.x - segment, bar.y, segment, bar_h), ink, bar_h / 2.0, 13);
            let from = window(p, bar, Edges::horizontal(segment / 2.0));
            let to = Props { x: bar.x + bar.w, ..from };
            p.plan.sweep = Some(Sweep { id: FILL, from, to, ms: (6 * p.tokens.motion.scroll_ms).max(600) });
        }
        None => {}
    }
    if p.model.can_cancel() {
        p.button(CANCEL, Focus::Cancel, "Cancel", Some(Icon::Back), x, cancel_y, false, true, 12);
    }
}

/// The launch's one line: what it is doing and, when the engine can say, how long it will take.
fn status(phase: &Phase) -> (String, Option<String>) {
    match phase {
        Phase::Queue { position, eta_s } => (
            position.map_or_else(|| "Waiting for a free machine".into(), |n| format!("You're {} in line", ordinal(n))),
            eta_s.map(wait),
        ),
        Phase::SignIn => ("Sign in to continue".into(), None),
        Phase::Starting => ("Getting your game ready".into(), None),
        Phase::Stopping => ("Closing the session".into(), None),
        _ => ("Checking the connection".into(), None),
    }
}

// shortcut: no thousands separator; a queue seldom passes four digits. Upgrade if one shows 12345th.
fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// An estimate of seconds, as the owner would say it.
fn wait(seconds: u32) -> String {
    if seconds < 60 { "less than a minute".into() } else { format!("about {} min", (seconds + 30) / 60) }
}

/// Where the baseline of `text` falls in its line box, as the glyph layer sets it.
fn baseline(p: &mut Paint<'_>, text: &str, role: Type) -> f32 {
    let ty = p.ty(role);
    let extent = p.fonts.measure(text, ty);
    ((line_height(ty) - extent.height()) / 2.0 + extent.ascent).round()
}

/// Shows the node just drawn only inside `rect`, fading over `feather` at its edges; returns the props that gives it.
fn window(p: &mut Paint<'_>, rect: Rect, feather: Edges) -> Props {
    let node = p.plan.nodes.last_mut().expect("the node just drawn");
    node.props = node.props.clip_feathered(rect, feather);
    node.props
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::{Input, Key, Model, tests::{launching, report}};
    use super::super::paint::{Body, Plan, assert_readable};
    use lumagame_core::ipc::SessionState;
    use lumatv_tokens::Tokens;
    use ui::{components::Form, lucent::palette::Mode};

    /// A title no game has: many words, and then one that fits no line.
    const LONG: &str = "The Extraordinarily Long-Winded Chronicles of Sir Bartholomew Fitzwilliam-Montgomery the Third and the Quest for the Unpronounceable Artefact of Everlasting Disappointment";
    const WORD: &str = "Supercalifragilisticexpialidocious-Hippopotomonstrosesquippedaliophobia-Pneumonoultramicroscopicsilicovolcanoconiosis";

    fn reports() -> Vec<SessionState> {
        let queue = |position, eta_s| SessionState::Queue { position, eta_s };
        vec![
            SessionState::Checking,
            SessionState::SignIn,
            queue(Some(12), Some(185)),
            queue(Some(1234), Some(7200)),
            queue(None, Some(45)),
            queue(None, None),
            SessionState::Starting,
        ]
    }
    fn page<R>(m: &Model, mode: Mode, fonts: &mut ui::Fonts, then: impl FnOnce(&mut Paint<'_>) -> R) -> R {
        let tokens = lumatv_tokens::themes::game_tokens("rose-pine", mode, &Tokens::shipped()).unwrap();
        let mut p = Paint::new(&tokens, m, fonts);
        launch(&mut p);
        p.fit_focus();
        then(&mut p)
    }
    fn union(plan: &Plan, ids: std::ops::RangeInclusive<u64>) -> Option<Frame> {
        plan.nodes.iter().filter(|n| ids.contains(&n.id)).map(|n| n.frame).reduce(|a, b| {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Frame::new(x, y, (a.x + a.w).max(b.x + b.w) - x, (a.y + a.h).max(b.y + b.h) - y)
        })
    }
    /// The blocks of the screen keep their distance from each other and from the screen's edge, and no icon touches its text.
    fn assert_clear(p: &mut Paint<'_>, what: &str) {
        assert_readable(&p.plan, p.fonts, true);
        let blocks: Vec<_> = [("identity", IDENTITY..=IDENTITY), ("steps", CHIPS..=CHIPS + 8), ("status", STATUS..=DETAIL), ("bar", TRACK..=TRACK), ("cancel", CANCEL..=CANCEL + 1)]
            .into_iter()
            .filter_map(|(name, ids)| union(&p.plan, ids).map(|f| (name, f)))
            .collect();
        assert!(blocks.len() >= 4, "{what}: a block is missing: {blocks:?}");
        for (name, f) in &blocks {
            assert!(f.x >= p.x - 1.0 && f.x + f.w <= W - p.x + 1.0 && f.y >= p.y - 1.0 && f.y + f.h <= H - p.y + 1.0, "{what}: {name} leaves the safe area: {f:?}");
        }
        for (i, (a, fa)) in blocks.iter().enumerate() {
            for (b, fb) in &blocks[..i] {
                let apart = (fa.y - (fb.y + fb.h)).max(fb.y - (fa.y + fa.h));
                assert!(apart >= 3.0, "{what}: {a} and {b} are {apart} apart: {fa:?} {fb:?}");
            }
        }
        for node in &p.plan.nodes {
            let Body::Text { words, marks } = &node.body else { continue };
            for mark in marks {
                let Form::Icon(_, side) = mark.form else { continue };
                for word in words {
                    let width = p.fonts.measure(&word.text, word.ty).width;
                    let apart = (word.x - (mark.x + side as f32)).max(mark.x - (word.x + width));
                    assert!(apart >= 2.0, "{what}: an icon touches {:?} ({apart} apart)", word.text);
                }
            }
        }
    }

    #[test]
    fn no_phase_crowds_or_overlaps_at_either_text_size_in_either_theme() {
        let mut fonts = ui::Fonts::new();
        for percent in [100, 200] {
            for mode in [Mode::Dark, Mode::Light] {
                let mut cases: Vec<(&str, bool, SessionState)> = reports().into_iter().map(|state| (LONG, false, state)).collect();
                cases.push((WORD, false, SessionState::Queue { position: Some(12), eta_s: Some(185) }));
                cases.push(("Hades", true, SessionState::Queue { position: Some(12), eta_s: Some(185) }));
                for (title, art, state) in cases {
                    let mut m = launching(title, art);
                    m.prefs.text_percent = percent;
                    report(&mut m, state.clone());
                    page(&m, mode, &mut fonts, |p| assert_clear(p, &format!("{percent}% {mode:?} {state:?} {title:.12}")));
                }
                // Mid-queue, with the bar full of what it has measured, and a launch being stopped.
                let mut m = launching(LONG, false);
                m.prefs.text_percent = percent;
                report(&mut m, SessionState::Queue { position: Some(12), eta_s: None });
                report(&mut m, SessionState::Queue { position: Some(3), eta_s: Some(185) });
                page(&m, mode, &mut fonts, |p| assert_clear(p, &format!("{percent}% {mode:?} measured queue")));
                m.step(Input::Key { key: Key::Back, down: true });
                m.step(Input::Key { key: Key::Back, down: false });
                page(&m, mode, &mut fonts, |p| assert_clear(p, &format!("{percent}% {mode:?} stopping")));
            }
        }
    }

    #[test]
    fn the_bar_follows_the_queue_and_otherwise_keeps_moving_unless_motion_is_reduced() {
        let mut fonts = ui::Fonts::new();
        let mut m = launching("Hades", false);
        let bar = |m: &Model, fonts: &mut ui::Fonts| {
            page(m, Mode::Dark, fonts, |p| {
                let track = p.plan.nodes.iter().find(|n| n.id == TRACK).map(|n| n.frame);
                let fill = p.plan.nodes.iter().find(|n| n.id == FILL).and_then(|n| n.props.clip);
                (track, fill, p.plan.sweep)
            })
        };
        let (track, fill, sweep) = bar(&m, &mut fonts);
        let (track, fill, sweep) = (track.unwrap(), fill.unwrap(), sweep.unwrap());
        assert_eq!(fill, Rect { x: track.x, y: track.y, w: track.w, h: track.h }, "the moving part never draws outside the track");
        assert!(sweep.from.x + 1.0 < track.x && sweep.to.x >= track.x + track.w, "it crosses the whole track, from outside one end to outside the other");

        report(&mut m, SessionState::Queue { position: Some(12), eta_s: None });
        report(&mut m, SessionState::Queue { position: Some(3), eta_s: None });
        let (track, fill, sweep) = bar(&m, &mut fonts);
        assert_eq!(sweep, None, "a measured wait does not sweep");
        assert_eq!(fill.unwrap().w, track.unwrap().w * 0.75, "nine of the twelve places ahead are behind");

        report(&mut m, SessionState::Queue { position: None, eta_s: None });
        assert!(bar(&m, &mut fonts).2.is_some(), "with no position there is nothing to measure");

        m.prefs.reduce_motion = true;
        let (track, _, sweep) = bar(&m, &mut fonts);
        assert!(track.is_none() && sweep.is_none(), "no bar rather than an empty one");
        report(&mut m, SessionState::Queue { position: Some(2), eta_s: None });
        let (track, fill, sweep) = bar(&m, &mut fonts);
        assert!(track.is_some() && fill.is_some() && sweep.is_none(), "a measured wait is still shown, still");
    }

    #[test]
    fn the_status_reads_in_plain_words() {
        assert_eq!(["1", "2", "3", "4", "11", "12", "13", "21", "112"].map(|n| ordinal(n.parse().unwrap())), ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "112th"]);
        assert_eq!([0, 59, 60, 185, 7200].map(wait), ["less than a minute", "less than a minute", "about 1 min", "about 3 min", "about 120 min"]);
        assert_eq!(status(&Phase::Queue { position: Some(12), eta_s: Some(185) }), ("You're 12th in line".into(), Some("about 3 min".into())));
        assert_eq!(status(&Phase::Starting).0, "Getting your game ready");
    }
}
