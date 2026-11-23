//! A drawer: an opaque sheet from one side with a scrim behind it.
//!
//! The sheet sits at the screen's edge insets and is as tall as the safe area. Its head holds the title; below it is the content the
//! caller fills, and under that the foot, where the layer's key hints go (`p.anchor`). The scrim covers the whole screen, as a dialog's
//! does, and the sheet lies over it.
//!
//! Node ids: 8800 the scrim, 8801 the sheet, 8802 the title.
use crate::game::screens::{
    paint::{Fill, H, Paint, W},
    parts::{SHEET_Z, Side, dialog::veil, motion::Enter},
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::geometry::Frame;

/// The one width of every side panel.
pub const WIDTH: f32 = 640.0;
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

/// Opens a drawer of `width` on `side` with `title`, and returns the frame for its content. Sets `p.art` off (the drawer is a sheet)
/// and `p.anchor` to the foot, so `hints::bar(.., Side::Left, ..)` sits inside the sheet.
pub fn open(p: &mut Paint<'_>, side: Side, width: f32, title: &str) -> Frame {
    p.art = false;
    let x = match side {
        Side::Left => p.x,
        Side::Right => W - p.x - width,
    };
    let sheet = Frame::new(x, p.y, width, H - 2.0 * p.y);
    let toward = if side == Side::Left { -SLIDE } else { SLIDE };

    p.arrives(Frame::new(0.0, 0.0, W, H), Enter::FADE);
    veil(p, 8800, Frame::new(0.0, 0.0, W, H), false);

    p.arrives(sheet, Enter::slide(toward, 0.0));
    p.surface(8801, sheet, Fill::Sheet, Radius::Sheet, false, false, false, SHEET_Z);
    let inner_x = sheet.x + PAD_SIDE + HEAD_SIDE;
    let inner_w = width - 2.0 * (PAD_SIDE + HEAD_SIDE);
    let head = HEAD_TOP + p.line(Type::Title) + HEAD_BOTTOM;
    let mut word = p.word(title, Type::Title, p.ink(ui::lucent::palette::PaletteRole::Ink, false), 0.0, 0.0, inner_w);
    word.ty = p.bold(Type::Title);
    p.words(8802, Frame::new(inner_x, sheet.y + PAD_TOP + HEAD_TOP, inner_w, p.line(Type::Title)), vec![word], vec![], SHEET_Z + 1);

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
    fn the_scrim_covers_the_screen_and_the_sheet_slides_in_from_its_edge() {
        painted(|_| {}, |p| {
            open(p, Side::Left, WIDTH, "Accounts");
            assert_eq!(frame_of(p, 8800), Frame::new(0.0, 0.0, W, H));
            p.finish();
            assert_eq!((p.plan.enters[&8801].dx, p.plan.enters[&8800].dx), (-SLIDE, 0.0));
        });
    }
}
