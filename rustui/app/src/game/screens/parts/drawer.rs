//! A drawer: an opaque sheet from one side with a scrim behind it.
//!
//! The sheet sits at the screen's edge insets and is as tall as the safe area. Its head holds the title, and over it a crumb back to the
//! view before when there is one; below it is the content the caller fills, and under that the foot, where the layer's key hints go
//! (`p.anchor`). The scrim covers only the screen beside the sheet: a full-screen layer under an opaque sheet would be drawn for nothing.
//! Over the art's pre-dimmed copy (`stage::over_art`) the page is dim already and there is no scrim.
//!
//! Node ids: 8800 the scrim, 8801 the sheet, 8802 the title, 8803 and 8804 the crumb.
use crate::game::screens::{
    paint::{Body, Fill, H, Paint, W},
    parts::{SHEET_Z, Side, dialog::veil, motion::Enter},
};
use crate::game::model::Focus;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

/// The one width of every side panel.
pub const WIDTH: f32 = 640.0;
/// The sheet's node id.
pub const SHEET: u64 = 8801;
/// The sheet comes in from this far toward its edge.
const SLIDE: f32 = 40.0;
/// Sheet padding: top, sides, bottom.
const PAD_TOP: f32 = 28.0;
const PAD_SIDE: f32 = 22.0;
const PAD_BOTTOM: f32 = 26.0;
/// The head's padding: top, sides, bottom.
const HEAD_TOP: f32 = 6.0;
const HEAD_SIDE: f32 = 28.0;
const HEAD_BOTTOM: f32 = 20.0;
/// The foot is this much clear space above the hints.
const FOOT_CLEAR: f32 = 18.0;
/// The crumb is 64 px tall; the title sits right under it.
const CRUMB_H: f32 = 64.0;

/// The scrim's node.
pub const SCRIM: u64 = 8800;

/// Opens a drawer of `width` on `side` with `title`, and returns the frame for its content. Sets `p.art` off (the drawer is a sheet),
/// `p.anchor` to the foot, so `hints::bar(.., Side::Left, ..)` sits inside the sheet, and `p.content_z` to the sheet's.
pub fn open(p: &mut Paint<'_>, side: Side, width: f32, title: &str) -> Frame {
    shell(p, side, width, None, title)
}

/// `open` with a crumb over the title, back to the view `crumb` names; focusing `crumb.1` lights it and the caller adds its hit.
pub fn open_crumbed(p: &mut Paint<'_>, side: Side, width: f32, crumb: (&str, Focus), title: &str) -> Frame {
    shell(p, side, width, Some(crumb), title)
}

/// Raises what was painted since node `from` onto the sheet.
pub fn lift(p: &mut Paint<'_>, from: usize) {
    for node in &mut p.plan.nodes[from..] {
        node.z += SHEET_Z;
    }
}

fn shell(p: &mut Paint<'_>, side: Side, width: f32, crumb: Option<(&str, Focus)>, title: &str) -> Frame {
    p.art = false;
    let x = match side {
        Side::Left => p.x,
        Side::Right => W - p.x - width,
    };
    let sheet = Frame::new(x, p.y, width, H - 2.0 * p.y);
    let toward = if side == Side::Left { -SLIDE } else { SLIDE };

    // Over the pre-dimmed art a scrim would dim the page twice.
    let dimmed = p.plan.nodes.iter().any(|n| matches!(&n.body, Body::Image(key) if key.dim()));
    if !dimmed {
        let beside = match side {
            Side::Left => Frame::new(sheet.x + width, 0.0, W - sheet.x - width, H),
            Side::Right => Frame::new(0.0, 0.0, sheet.x, H),
        };
        p.arrives(beside, Enter::FADE);
        veil(p, SCRIM, beside, false);
    }

    p.arrives(sheet, Enter::slide(toward, 0.0));
    p.surface(SHEET, sheet, Fill::Sheet, Radius::Sheet, false, false, false, SHEET_Z);
    p.content_z = SHEET_Z;
    let inner_x = sheet.x + PAD_SIDE + HEAD_SIDE;
    let inner_w = width - 2.0 * (PAD_SIDE + HEAD_SIDE);
    let mut title_y = sheet.y + PAD_TOP + HEAD_TOP;
    if let Some((name, focus)) = crumb {
        let h = CRUMB_H.max(p.tokens.target);
        let icon = 28.0 * p.scale;
        let ty = p.bold(Type::Label);
        let text_w = p.fonts.measure(name, ty).width.ceil();
        let f = Frame::new(inner_x, title_y, 24.0 + icon + 12.0 + text_w + 28.0, h);
        let focused = p.focused(focus);
        let from = p.plan.nodes.len();
        // Uncached: a cached glass pill appearing after the page's first frames would capture a backdrop.
        p.surface(8803, f, Fill::Glass, Radius::Pill, false, focused, false, SHEET_Z + 1);
        let mut word = p.word(name, Type::Label, p.ink(Role::Ink, false), 24.0 + icon + 12.0, (h - p.line(Type::Label)) / 2.0, text_w);
        word.ty = ty;
        let back = p.mark(Icon::Back, icon, 24.0, (h - icon) / 2.0, Role::Ink, false);
        p.words(8804, f, vec![word], vec![back], SHEET_Z + 2);
        if focused {
            p.zoom(from, f, p.tokens.focus.button_scale);
        }
        title_y += h;
    }
    let head = title_y - (sheet.y + PAD_TOP) + p.line(Type::Title) + HEAD_BOTTOM;
    let mut word = p.word(title, Type::Title, p.ink(Role::Ink, false), 0.0, 0.0, inner_w);
    word.ty = p.bold(Type::Title);
    p.words(8802, Frame::new(inner_x, title_y, inner_w, p.line(Type::Title)), vec![word], vec![], SHEET_Z + 1);

    let foot_h = FOOT_CLEAR + 37.0 * p.scale;
    let foot_y = sheet.y + sheet.h - PAD_BOTTOM - foot_h;
    p.anchor = Frame::new(inner_x, foot_y, inner_w, foot_h);
    let top = sheet.y + PAD_TOP + head;
    Frame::new(sheet.x + PAD_SIDE, top, width - 2.0 * PAD_SIDE, foot_y - top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::parts::painted;

    fn frame_of(p: &Paint<'_>, id: u64) -> Frame {
        p.plan.nodes.iter().find(|n| n.id == id).unwrap().frame
    }

    #[test]
    fn the_sheet_sits_at_the_edge_inset_and_the_content_lies_between_head_and_foot() {
        painted(|_| {}, |p| {
            p.art = true;
            let content = open(p, Side::Right, WIDTH, "Settings");
            let sheet = frame_of(p, 8801);
            assert!(!p.art);
            assert_eq!((sheet.x + sheet.w, sheet.y, sheet.h), (W - p.x, p.y, H - 2.0 * p.y));
            assert!(content.y > frame_of(p, 8802).y && content.y + content.h <= p.anchor.y + 0.01);
            assert!(content.x >= sheet.x && content.x + content.w <= sheet.x + sheet.w);
            assert!(p.anchor.y + p.anchor.h <= sheet.y + sheet.h);
        });
    }

    #[test]
    fn the_scrim_lies_beside_the_sheet_and_the_sheet_slides_in_from_its_edge() {
        painted(|_| {}, |p| {
            open(p, Side::Left, WIDTH, "Accounts");
            let sheet = frame_of(p, 8801);
            assert_eq!(frame_of(p, 8800), Frame::new(sheet.x + sheet.w, 0.0, W - sheet.x - sheet.w, H));
            p.finish();
            assert_eq!((p.plan.enters[&8801].dx, p.plan.enters[&8800].dx), (-SLIDE, 0.0));
        });
    }
}
