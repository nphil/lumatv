//! Glance cards: a label, a headline, a line of detail, a progress bar, and at the foot a caption row over some thumbnails and where the facts
//! came from. A row of them is the game page's strip of sections.
//!
//! Node ids: `Card::id` and the 15 after it: the sheet, then the label, headline, detail, track, fill, caption and source, the thumbnails
//! from `id + 8`, and the focus treatment from `id + 12`. The card registers no hit: the page that places it knows what it opens.
use crate::game::screens::{
    paint::{Fill, Paint},
    parts::focus::{self, Kind},
};
use lumatv_tokens::{Radius, TypeRole as Type};
use std::path::PathBuf;
use ui::{geometry::Frame, components::Icon, lucent::palette::PaletteRole as Role};

/// How a card's headline reads: a figure or a word in the title face, or a sentence in the body face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Headline<'a> {
    /// `12 of 33`, `Very positive`: one line.
    Big(&'a str),
    /// A news item's title: up to two lines.
    Text(&'a str),
}

/// Where a card's facts came from, and how old they are when they are not fresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    pub name: &'a str,
    pub stale: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct Card<'a> {
    pub id: u64,
    pub label: &'a str,
    pub headline: Headline<'a>,
    pub detail: Option<&'a str>,
    /// 0 to 1.
    pub progress: Option<f32>,
    pub thumbs: &'a [PathBuf],
    /// Small words on a row of their own above the foot's thumbnails or source line; left out when the card has no room for the row.
    pub caption: Option<&'a str>,
    pub source: Option<Source<'a>>,
}

const PAD_X: f32 = 30.0;
const PAD_Y: f32 = 28.0;
const THUMB: f32 = 64.0;
const THUMB_GAP: f32 = 12.0;
/// A card shows this many thumbnails at most.
const THUMBS: usize = 4;
const TRACK_H: f32 = 8.0;
/// The room kept between the card's body and its foot.
const FOOT_GAP: f32 = 12.0;
const Z: i32 = 10;

/// The card, painted into `frame`; `focused` lights it (see [`focus::treat`]).
pub fn glance(p: &mut Paint<'_>, card: &Card<'_>, frame: Frame, focused: bool) {
    let from = p.plan.nodes.len();
    let id = card.id;
    p.surface(id, frame, Fill::Sheet, Radius::Card, false, false, false, Z);
    let (x, w) = (frame.x + PAD_X, frame.w - 2.0 * PAD_X);
    // What is read in ink-2 at rest reads in ink once lit: the wash lowers the contrast of the quieter colour.
    let quiet = if focused { Role::Ink } else { Role::Ink2 };
    let side = 24.0 * p.scale;

    let mut y = frame.y + PAD_Y;
    let label = p.fit(card.label, Type::Caption, w, 1);
    y += put(p, id + 1, label, Frame::new(x, y, w, 0.0), Type::Caption, false, quiet);
    let (headline, role, bold, gap) = match card.headline {
        Headline::Big(text) => (p.fit(text, Type::Title, w, 1), Type::Title, true, 6.0),
        Headline::Text(text) => (p.fit(text, Type::Body, w, 2), Type::Body, true, 8.0),
    };
    y += gap;
    y += put(p, id + 2, headline, Frame::new(x, y, w, 0.0), role, bold, Role::Ink);
    // Large text leaves a short card little room under its headline: the thumbnails, then the track, then the detail give way
    // rather than overlap it.
    let bottom = frame.y + frame.h - PAD_Y;
    let line = p.line(Type::Caption);
    let caption = card.caption.filter(|_| y + FOOT_GAP + line <= bottom);
    let caption_h = caption.map_or(0.0, |_| line + FOOT_GAP);
    let thumbs = if y <= bottom - THUMB - FOOT_GAP - caption_h { card.thumbs } else { &[] };
    let row_h = if !thumbs.is_empty() {
        THUMB
    } else if card.source.is_some() {
        line.max(side)
    } else {
        0.0
    };
    let foot_h = row_h + caption_h;
    let foot_y = bottom - foot_h;
    let row_y = bottom - row_h;
    let floor = foot_y - if foot_h > 0.0 { FOOT_GAP } else { 0.0 };
    if let Some(done) = card.progress.filter(|_| y + 18.0 + TRACK_H <= floor) {
        y += 18.0;
        let track = Frame::new(x, y, w, TRACK_H);
        p.surface(id + 4, track, Fill::Track, Radius::Pill, false, false, false, Z + 1);
        let ink = p.ink(Role::Ink, false);
        p.solid(id + 5, Frame::new(x, y, (w * done.clamp(0.0, 1.0)).max(TRACK_H), TRACK_H), ink, TRACK_H / 2.0, Z + 2);
        y += TRACK_H;
    }
    if let Some(detail) = card.detail.filter(|_| floor - (y + 8.0) >= p.block(Type::Label, 1)) {
        y += 8.0;
        let lines = p.fit(detail, Type::Label, w, p.rows(Type::Label, floor - y).min(3));
        put(p, id + 3, lines, Frame::new(x, y, w, 0.0), Type::Label, false, quiet);
    }

    // The source line is whole or absent: thumbnails make room for it before it gives way to an ellipsis.
    let source_w = card.source.map_or(0.0, |s| {
        let text = if s.stale.is_some() { format!("From {}, {}", s.name, s.stale.unwrap_or_default()) } else { format!("From {}", s.name) };
        p.width(&text, Type::Caption) + if s.stale.is_some() { side + 8.0 } else { 0.0 } + THUMB_GAP
    });
    let fit = ((w - source_w + THUMB_GAP) / (THUMB + THUMB_GAP)).floor().max(0.0) as usize;
    let mut left = x;
    for (i, path) in thumbs.iter().take(THUMBS.min(fit)).enumerate() {
        let radius = p.tokens.radius(Radius::Control);
        p.image(id + 8 + i as u64, path.clone(), Frame::new(left, row_y, THUMB, THUMB), radius, false, None, Z + 1);
        left += THUMB + THUMB_GAP;
    }
    if let Some(caption) = caption {
        let lines = p.fit(caption, Type::Caption, w, 1);
        put(p, id + 6, lines, Frame::new(x, foot_y, w, 0.0), Type::Caption, false, quiet);
    }
    if let Some(source) = card.source {
        let text = match source.stale {
            Some(age) => format!("From {}, {age}", source.name),
            None => format!("From {}", source.name),
        };
        let mark = if source.stale.is_some() { side + 8.0 } else { 0.0 };
        let reach = (x + w - left).max(0.0);
        let lines = p.fit(&text, Type::Caption, (reach - mark).max(0.0), 1);
        let text_w = p.width(&lines[0], Type::Caption);
        let at = x + w - mark - text_w;
        let word = p.word(&lines[0], Type::Caption, p.ink(quiet, false), mark, (side - p.line(Type::Caption)) / 2.0, text_w);
        let marks = if source.stale.is_some() { vec![p.mark(Icon::Alert, side, 0.0, 0.0, Role::Warning, false)] } else { Vec::new() };
        let row = Frame::new(at, row_y + (row_h - side) / 2.0, mark + text_w, side);
        p.words(id + 7, row, vec![word], marks, Z + 1);
    }
    focus::treat(p, from, id + 12, frame, Kind::Card, focused);
}

/// `lines` of `role` from the origin of `at`, as wide as it; the height they take.
fn put(p: &mut Paint<'_>, id: u64, lines: Vec<String>, at: Frame, role: Type, bold: bool, colour: Role) -> f32 {
    let Frame { x, y, w, .. } = at;
    let line = p.line(role);
    let height = lines.len() as f32 * line;
    let ty = if bold { p.bold(role) } else { p.ty(role) };
    let ink = p.ink(colour, false);
    let words = lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let mut word = p.word(text, role, ink, 0.0, i as f32 * line, w);
            word.ty = ty;
            word
        })
        .collect::<Vec<_>>();
    let drawn = words.last().map_or(0.0, |last| last.y + ui::components::line_height(ty).max(line));
    p.words(id, Frame::new(x, y, w, drawn), words, Vec::new(), Z + 1);
    height
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::{paint::Body, parts::painted};

    const FRAME: Frame = Frame { x: 100.0, y: 200.0, w: 520.0, h: 330.0 };

    fn card<'a>(thumbs: &'a [PathBuf]) -> Card<'a> {
        Card {
            id: 7000,
            label: "Achievements",
            headline: Headline::Big("12 of 33"),
            detail: Some("Latest: Tipping the scales"),
            progress: Some(0.36),
            thumbs,
            caption: None,
            source: Some(Source { name: "Steam", stale: None }),
        }
    }
    fn frame_of(p: &Paint<'_>, id: u64) -> Frame {
        p.plan.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("node {id}")).frame
    }
    fn texts(p: &Paint<'_>, id: u64) -> Vec<String> {
        match &p.plan.nodes.iter().find(|n| n.id == id).unwrap().body {
            Body::Text { words, .. } => words.iter().map(|w| w.text.clone()).collect(),
            _ => panic!("node {id} is not text"),
        }
    }

    #[test]
    fn a_card_reads_top_down_with_its_foot_on_the_bottom_padding() {
        let thumbs = [PathBuf::from("a.png"), PathBuf::from("b.png"), PathBuf::from("c.png")];
        painted(|_| {}, |p| {
            glance(p, &card(&thumbs), FRAME, false);
            let (label, headline, track, detail) = (frame_of(p, 7001), frame_of(p, 7002), frame_of(p, 7004), frame_of(p, 7003));
            assert!(label.y < headline.y && headline.y + headline.h <= track.y && track.y + track.h < detail.y);
            assert_eq!((label.x, label.w), (FRAME.x + PAD_X, FRAME.w - 2.0 * PAD_X));
            let (first, last) = (frame_of(p, 7008), frame_of(p, 7010));
            assert_eq!((first.w, first.h, first.y + first.h), (THUMB, THUMB, FRAME.y + FRAME.h - PAD_Y), "thumbnails sit on the padding");
            assert_eq!(last.x, first.x + 2.0 * (THUMB + THUMB_GAP));
            assert_eq!(texts(p, 7007), ["From Steam"]);
            let source = frame_of(p, 7007);
            assert!((source.x + source.w - (FRAME.x + FRAME.w - PAD_X)).abs() < 0.01, "the source ends at the right padding");
            assert!(detail.y + detail.h <= first.y, "the detail stays clear of the foot");
        });
    }

    #[test]
    fn a_short_card_lets_thumbnails_track_and_detail_go_before_they_overlap() {
        let thumbs = [PathBuf::from("a.png")];
        painted(|_| {}, |p| {
            glance(p, &card(&thumbs), Frame { h: 200.0, ..FRAME }, false);
            let has = |id: u64| p.plan.nodes.iter().any(|n| n.id == id);
            assert!(!has(7008) && !has(7004) && !has(7003), "thumbnail, track and detail give way");
            let (headline, source) = (frame_of(p, 7002), frame_of(p, 7007));
            assert!(headline.y + headline.h <= source.y, "the headline stays clear of the foot");
        });
    }

    #[test]
    fn a_caption_has_its_own_row_clear_of_thumbnails_and_the_text_above_at_both_text_sizes() {
        let thumbs = [PathBuf::from("a.png"), PathBuf::from("b.png"), PathBuf::from("c.png"), PathBuf::from("d.png")];
        for percent in [100, 200] {
            painted(|m| m.prefs.text_percent = percent, |p| {
                let mut with = card(&thumbs);
                with.caption = Some("A long news headline about the next update to the game");
                let frame = Frame { h: 330.0 * p.scale, ..FRAME };
                glance(p, &with, frame, false);
                let caption = frame_of(p, 7006);
                assert_eq!(caption.x, frame.x + PAD_X, "{percent}%: the row starts at the padding");
                let above = [7002, 7003].into_iter().filter(|id| p.plan.nodes.iter().any(|n| n.id == *id)).map(|id| frame_of(p, id));
                for text in above {
                    assert!(text.y + text.h <= caption.y, "{percent}%: clear of the text above");
                }
                if let Some(node) = p.plan.nodes.iter().find(|n| n.id == 7008) {
                    assert!(caption.y + caption.h <= node.frame.y, "{percent}%: clear of the thumbnails");
                }
                assert!(caption.y + caption.h <= frame.y + frame.h - PAD_Y, "{percent}%: inside the card");
            });
        }
    }

    #[test]
    fn thumbnails_make_room_for_the_whole_source_line_in_a_narrow_card() {
        let thumbs = [PathBuf::from("a.png"), PathBuf::from("b.png"), PathBuf::from("c.png"), PathBuf::from("d.png")];
        painted(|_| {}, |p| {
            glance(p, &card(&thumbs), Frame { w: 400.0, ..FRAME }, false);
            assert_eq!(texts(p, 7007), ["From Steam"]);
            let last = (8..12).rev().find_map(|i| p.plan.nodes.iter().find(|n| n.id == 7000 + i)).unwrap().frame;
            let source = frame_of(p, 7007);
            assert!(last.x + last.w <= source.x, "no thumbnail under the source");
            assert!(!p.plan.nodes.iter().any(|n| n.id == 7011), "the fourth thumbnail gave way");
        });
    }

    #[test]
    fn a_stale_source_says_how_old_and_wears_a_warning_mark() {
        painted(|_| {}, |p| {
            let mut stale = card(&[]);
            stale.source = Some(Source { name: "Steam", stale: Some("2 days ago") });
            glance(p, &stale, FRAME, false);
            assert_eq!(texts(p, 7007), ["From Steam, 2 days ago"]);
            let Body::Text { marks, .. } = &p.plan.nodes.iter().find(|n| n.id == 7007).unwrap().body else { panic!() };
            assert_eq!(marks.len(), 1);
        });
    }

    #[test]
    fn a_card_without_progress_detail_or_foot_is_just_label_and_headline() {
        painted(|_| {}, |p| {
            let bare = Card { id: 7000, label: "Reviews", headline: Headline::Text("Very positive"), detail: None, progress: None, thumbs: &[], caption: None, source: None };
            glance(p, &bare, FRAME, false);
            assert_eq!(p.plan.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [7000, 7001, 7002]);
        });
    }

    #[test]
    fn a_focused_card_is_lit_and_reads_its_quiet_lines_in_ink() {
        painted(|_| {}, |p| {
            glance(p, &card(&[]), FRAME, false);
            let rest = match &p.plan.nodes.iter().find(|n| n.id == 7001).unwrap().body {
                Body::Text { words, .. } => words[0].colour,
                _ => unreachable!(),
            };
            p.plan.nodes.clear();
            glance(p, &card(&[]), FRAME, true);
            let lit = match &p.plan.nodes.iter().find(|n| n.id == 7001).unwrap().body {
                Body::Text { words, .. } => words[0].colour,
                _ => unreachable!(),
            };
            assert_ne!(rest, lit);
            assert!(p.plan.nodes.iter().any(|n| n.id == 7012) && !p.plan.nodes.iter().any(|n| n.id == 7013), "the wash and no underline");
        });
    }
}
