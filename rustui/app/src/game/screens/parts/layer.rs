//! The layer drawn over the page: picks the open layer's painter, and the reader layer a section page is (`section`).
//!
//! Node ids of `section`: 8840 the scrim, 8842 the sheet, 8850 and up each tab's wash, 8860 and up
//! each tab's name.
use crate::game::screens::paint::{Fill, H, Paint, W};
use crate::game::model::{Layer, library::Sort};
use crate::game::screens::parts::{SHEET_Z, dialog::*, keyboard::*, motion::Enter};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::{accounts::auth, game::chooser, launch::end_session, library::list, settings::{delete, themes}};

pub fn sheet(p: &mut Paint<'_>) {
    let Some(sheet) = p.model.view.top().filter(|layer| layer.is_sheet()) else {
        return;
    };
    curtain(p);
    match sheet {
        Layer::Chooser => chooser(p),
        Layer::Sort => list(p, "Sort by", Sort::ALL.iter().map(|s| s.name().into()).collect()),
        Layer::Genre => {
            let mut names = vec!["All genres".into()];
            names.extend(p.model.genres());
            list(p, "Genre", names);
        }
        Layer::Theme => themes(p),
        Layer::Auth => auth(p),
        Layer::EndSession => end_session(p),
        Layer::AddHost | Layer::EditText => edit_text(p),
        Layer::Delete => delete(p),
        // Drawn by their page in place of its own content.
        Layer::Viewer | Layer::StreamMenu => {}
    }
}

/// The reader layer is this wide.
const READER_W: f32 = 1080.0;
/// The sheet comes up from this far below.
const RISE: f32 = 24.0;
/// Sheet padding at the top and the sides, and clear space under the content, which holds the hints.
const PAD_TOP: f32 = 28.0;
const PAD_SIDE: f32 = 22.0;
const PAD_BOTTOM: f32 = 26.0;
/// The tabs sit this far in from the sheet's padding; a tab is at least 64 px tall with this room each side of its name.
const TABS_SIDE: f32 = 14.0;
const TAB_H: f32 = 64.0;
const TAB_PAD: f32 = 30.0;
/// The content starts this far under the tabs.
const BODY_TOP: f32 = 26.0;
/// The foot is this much clear space above the hints.
const FOOT_CLEAR: f32 = 18.0;

/// Opens the reader layer on the right: a tab row with a name for each of `sections` (`current` in the selected wash) and, under it,
/// the slot for the section's content. The shell comes up as one; the content fades in on the spot, which is why the caller should give
/// each section's nodes ids of their own. Sets `p.art` off and `p.anchor` to the foot, for the hints. Returns the content slot.
pub fn section(p: &mut Paint<'_>, sections: &[&str], current: usize) -> Frame {
    p.art = false;
    let sheet = Frame::new(W - p.x - READER_W, p.y, READER_W, H - 2.0 * p.y);

    p.arrives(Frame::new(0.0, 0.0, W, H), Enter::FADE);
    veil(p, 8840, Frame::new(0.0, 0.0, W, H), false);

    p.arrives(sheet, Enter::slide(0.0, RISE));
    p.surface(8842, sheet, Fill::Sheet, Radius::Sheet, false, false, false, SHEET_Z);
    let tabs_y = sheet.y + PAD_TOP;
    let h = TAB_H.max(p.tokens.target);
    let mut x = sheet.x + PAD_SIDE + TABS_SIDE;
    for (i, name) in sections.iter().enumerate() {
        let w = p.width(name, Type::Label) + 2.0 * TAB_PAD;
        let tab = Frame::new(x, tabs_y, w, h);
        let ink = if i == current {
            p.surface(8850 + i as u64, tab, Fill::Tint(p.tokens.states.selected_wash), Radius::Pill, false, false, false, SHEET_Z + 1);
            Role::Ink
        } else {
            Role::Ink2
        };
        let mut word = p.word(*name, Type::Label, p.ink(ink, false), TAB_PAD, (h - p.line(Type::Label)) / 2.0, w - 2.0 * TAB_PAD);
        word.ty = p.bold(Type::Label);
        p.words(8860 + i as u64, tab, vec![word], vec![], SHEET_Z + 2);
        x += w;
    }

    let foot_h = FOOT_CLEAR + 37.0 * p.scale;
    let foot_y = sheet.y + sheet.h - PAD_BOTTOM - foot_h;
    p.anchor = Frame::new(sheet.x + PAD_SIDE + TABS_SIDE, foot_y, READER_W - 2.0 * (PAD_SIDE + TABS_SIDE), foot_h);
    let top = tabs_y + h + BODY_TOP;
    let slot = Frame::new(sheet.x + PAD_SIDE, top, READER_W - 2.0 * PAD_SIDE, foot_y - top);
    p.arrives(slot, Enter::CONTENT);
    slot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::parts::painted;

    fn frame_of(p: &Paint<'_>, id: u64) -> Frame {
        p.plan.nodes.iter().find(|n| n.id == id).unwrap().frame
    }

    #[test]
    fn the_reader_sits_on_the_right_with_its_slot_between_tabs_and_hints() {
        painted(|_| {}, |p| {
            p.art = true;
            let slot = section(p, &["Overview", "Controls", "Streaming"], 1);
            let sheet = frame_of(p, 8842);
            assert!(!p.art);
            assert_eq!((sheet.w, sheet.x + sheet.w), (READER_W, W - p.x));
            assert!(slot.y > frame_of(p, 8860).y + TAB_H - 0.01 && slot.y + slot.h <= p.anchor.y + 0.01);
            assert_eq!(frame_of(p, 8861).x, frame_of(p, 8860).x + frame_of(p, 8860).w);
        });
    }

    #[test]
    fn only_the_current_tab_is_washed_and_the_content_fades_where_the_shell_slides() {
        painted(|_| {}, |p| {
            let slot = section(p, &["A", "B"], 1);
            p.solid(9000, slot, p.ink(Role::Ink, false), 0.0, SHEET_Z + 1);
            p.finish();
            let ids: Vec<u64> = p.plan.nodes.iter().map(|n| n.id).collect();
            assert!(ids.contains(&8851) && !ids.contains(&8850));
            assert_eq!((p.plan.enters[&8842].dy, p.plan.enters[&8860].dy), (RISE, RISE));
            assert_eq!((p.plan.enters[&9000].dy, p.plan.enters[&9000].fade_ms), (0.0, Some(crate::game::screens::parts::motion::CONTENT_MS)));
        });
    }
}
