//! The layer drawn over the page: picks the open layer's painter, and the reader layer a section page is (`section`).
//!
//! Node ids of `section`: 8842 the sheet, 8850 the current tab's wash, 8860 the tabs' names.
use crate::game::screens::paint::{Fill, H, Paint, W};
use crate::game::model::Layer;
use crate::game::screens::parts::{SHEET_Z, dialog::*, keyboard::*, motion::Enter};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::{accounts::{auth, sign_out}, quick::quick, library::refine, settings::delete};

pub fn sheet(p: &mut Paint<'_>) {
    let Some(sheet) = p.model.view.top().filter(|layer| layer.is_sheet()) else {
        return;
    };
    // The quick menu's drawer keeps the page in view beside it, the Library's Refine drawer too, and a dialog brings its own backdrop.
    if !matches!(sheet, Layer::Quick | Layer::QuickSettings | Layer::Auth | Layer::SignOut | Layer::Delete | Layer::Sort | Layer::Genre | Layer::PlaysOn) {
        curtain(p);
    }
    match sheet {
        Layer::Sort | Layer::Genre | Layer::PlaysOn => refine(p, sheet),
        Layer::SignOut => sign_out(p),
        Layer::Auth => auth(p),
        Layer::AddHost | Layer::EditText => edit_text(p),
        Layer::Delete => delete(p),
        Layer::Quick | Layer::QuickSettings => quick(p),
        // Drawn by their page in place of, or over, its own content.
        Layer::Chooser | Layer::Section(_) | Layer::Reader | Layer::Viewer | Layer::Picture | Layer::Achievements | Layer::EndConfirm => {}
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
/// the slot for the section's content. The shell comes up as one; the content fades in on the spot and goes at once, which is why the
/// caller should give each section's nodes ids of their own: a switch then never holds two sections' content. It draws no scrim: the
/// page beneath dims itself (the pre-dimmed art). Sets `p.art` off, `p.anchor` to the foot, for the hints, and `p.content_z` to the
/// sheet's. Returns the content slot.
pub fn section(p: &mut Paint<'_>, sections: &[&str], current: usize) -> Frame {
    p.art = false;
    let sheet = Frame::new(W - p.x - READER_W, p.y, READER_W, H - 2.0 * p.y);
    p.arrives(sheet, Enter::slide(0.0, RISE));
    p.surface(8842, sheet, Fill::Sheet, Radius::Sheet, false, false, false, SHEET_Z);
    p.content_z = SHEET_Z;
    let mut tabs = Frame::new(sheet.x + PAD_SIDE + TABS_SIDE, sheet.y + PAD_TOP, 0.0, TAB_H.max(p.tokens.target));
    let ty = p.bold(Type::Label);
    let mut words = Vec::with_capacity(sections.len());
    for (i, name) in sections.iter().enumerate() {
        let w = p.width(name, Type::Label) + 2.0 * TAB_PAD;
        let ink = if i == current {
            let wash = Frame::new(tabs.x + tabs.w, tabs.y, w, tabs.h);
            p.surface(8850, wash, Fill::Tint(p.tokens.states.selected_wash), Radius::Pill, false, false, false, SHEET_Z + 1);
            Role::Ink
        } else {
            Role::Ink2
        };
        let mut word = p.word(*name, Type::Label, p.ink(ink, false), tabs.w + TAB_PAD, (tabs.h - p.line(Type::Label)) / 2.0, w - 2.0 * TAB_PAD);
        word.ty = ty;
        words.push(word);
        tabs.w += w;
    }
    // One layer for every name: the wash alone moves when the section changes.
    p.words(8860, tabs, words, vec![], SHEET_Z + 2);

    let foot_h = FOOT_CLEAR + 37.0 * p.scale;
    let foot_y = sheet.y + sheet.h - PAD_BOTTOM - foot_h;
    p.anchor = Frame::new(sheet.x + PAD_SIDE + TABS_SIDE, foot_y, READER_W - 2.0 * (PAD_SIDE + TABS_SIDE), foot_h);
    let top = tabs.y + tabs.h + BODY_TOP;
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
            let (tabs, wash) = (frame_of(p, 8860), frame_of(p, 8850));
            assert!(slot.y > tabs.y + TAB_H - 0.01 && slot.y + slot.h <= p.anchor.y + 0.01);
            assert_eq!(wash.x, tabs.x + p.width("Overview", Type::Label) + 2.0 * TAB_PAD, "the second tab is washed");
            assert_eq!(p.content_z, SHEET_Z);
        });
    }

    #[test]
    fn the_current_tab_is_washed_and_the_content_fades_where_the_shell_slides() {
        painted(|_| {}, |p| {
            let slot = section(p, &["A", "B"], 1);
            p.solid(9000, slot, p.ink(Role::Ink, false), 0.0, SHEET_Z + 1);
            p.finish();
            assert_eq!(p.plan.nodes.iter().filter(|n| n.id == 8850).count(), 1);
            assert_eq!((p.plan.enters[&8842].dy, p.plan.enters[&8860].dy), (RISE, RISE));
            assert_eq!((p.plan.enters[&9000].dy, p.plan.enters[&9000].fade_ms), (0.0, Some(crate::game::screens::parts::motion::CONTENT_MS)));
        });
    }
}
