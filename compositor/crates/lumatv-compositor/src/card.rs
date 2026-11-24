//! The picture kept for an app whose own pixels are no picture of it: its window is video on the plane (DRM, or a hole over most of the
//! screen), which a copy of the window cannot hold. The theme's colours and the app's name, drawn at the size of the other pictures.
//!
//! Panel care: the card covers a reopen for at most `RESUME_CAP_NS` (1.5 s) and dissolves in `FADE_NS`; it is a dim panel fill with one
//! tile and a title, about 1% of the screen lit, and it does not move beyond the open motion.

use crate::Rect;
use crate::snapshot::{PICTURE_HEIGHT, PICTURE_WIDTH};
use crate::taskmgr::Palette;
use crate::taskmgr::fallback::initial_of;
use crate::taskmgr::font::{Canvas, Typeface, mix, opaque};

/// The tile the initial sits on, and its corner radius.
const TILE: i32 = 176;
const TILE_RADIUS: f32 = 44.0;
/// How far the tile is mixed from the panel towards the text colour: a raised sheet, not a lit one.
const TILE_LIFT: f32 = 0.10;
const INITIAL_PX: u32 = 104;
const TITLE_PX: u32 = 28;
const TITLE_GAP: i32 = 34;
const TITLE_MAX: i32 = 720;
/// The tile's top edge: the tile and the title under it sit about the middle of the picture.
const TILE_TOP: i32 = 150;

/// What the card calls the app: its window's title, else its id, as the task manager's cards do.
fn label<'a>(title: &'a str, app: &'a str) -> &'a str {
    match title.trim() {
        "" => app,
        t => t,
    }
}

/// The card of the app `app` (window title `title`), premultiplied RGBA, top row first, `PICTURE_WIDTH` by `PICTURE_HEIGHT`.
pub(crate) fn paint(palette: &Palette, title: &str, app: &str) -> Vec<u8> {
    let (w, h) = (PICTURE_WIDTH as i32, PICTURE_HEIGHT as i32);
    let mut canvas = Canvas::new(Rect::new(0, 0, w, h));
    canvas.rounded_rect(canvas.rect(), 0.0, opaque(palette.panel));
    let mut face = Typeface::new();
    let tile = Rect::new((w - TILE) / 2, TILE_TOP, TILE, TILE);
    canvas.rounded_rect(tile, TILE_RADIUS, opaque(mix(palette.panel, palette.text, TILE_LIFT)));
    let name = label(title, app);
    // The letter the task manager's own card shows for the app (`taskmgr/fallback.rs`), so a card is the same app in both places.
    let letter = initial_of(title, app).to_string();
    let width = face.measure(&letter, INITIAL_PX);
    // The cap height is about 0.72 of the size: the letter's middle, not its line's, goes on the tile's.
    let top = tile.y + TILE / 2 - face.ascent(INITIAL_PX) + (INITIAL_PX as i32 * 36 / 100);
    canvas.text(&mut face, &letter, (tile.x + (TILE - width) / 2, top), INITIAL_PX, opaque(palette.text), None);
    let shown = face.fit(name, TITLE_PX, TITLE_MAX).into_owned();
    let width = face.measure(&shown, TITLE_PX);
    canvas.text(&mut face, &shown, ((w - width) / 2, tile.bottom() + TITLE_GAP), TITLE_PX, opaque(palette.muted), None);
    canvas.into_rgba()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(px: &[u8], x: i32, y: i32) -> [u8; 4] {
        let i = (y as usize * PICTURE_WIDTH + x as usize) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    fn differs(px: &[u8], r: Rect, from: [u8; 3]) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| at(px, x, y)[..3] != from))
    }

    #[test]
    fn the_card_is_an_opaque_picture_of_the_pictures_size() {
        let px = paint(&Palette::default(), "Netflix", "netflix");
        assert_eq!(px.len(), crate::snapshot::PICTURE_BYTES);
        assert!(px.chunks_exact(4).all(|p| p[3] == 255), "no hole in it: nothing under a card shows through");
    }

    #[test]
    fn the_panel_the_tile_the_initial_and_the_title_take_the_themes_colours() {
        let p = Palette { panel: [10, 20, 30], text: [240, 230, 220], muted: [120, 130, 140], accent: [1, 1, 1], danger: [2, 2, 2] };
        let px = paint(&p, "Netflix", "netflix");
        assert_eq!(at(&px, 0, 0)[..3], p.panel);
        assert_eq!(at(&px, 959, 539)[..3], p.panel);
        let tile = Rect::new((PICTURE_WIDTH as i32 - TILE) / 2, TILE_TOP, TILE, TILE);
        assert_eq!(at(&px, tile.x + 8, tile.y + TILE / 2)[..3], mix(p.panel, p.text, TILE_LIFT), "the raised sheet is the panel moved towards the text");
        assert_eq!(at(&px, tile.x + 1, tile.y + 1)[..3], p.panel, "the tile's corner is rounded away");
        let letter = Rect::new(tile.x + 40, tile.y + 30, TILE - 80, TILE - 60);
        assert!(differs(&px, letter, mix(p.panel, p.text, TILE_LIFT)), "the initial is drawn on the tile");
        let title = Rect::new(200, tile.bottom() + TITLE_GAP, 560, 30);
        assert!(differs(&px, title, p.panel), "the title is drawn under it");
    }

    #[test]
    fn no_colour_of_another_theme_is_left_in_the_card() {
        let dark = Palette::default();
        let light = Palette { panel: [250, 244, 237], text: [87, 82, 121], muted: [121, 117, 147], accent: [144, 122, 169], danger: [180, 99, 122] };
        let (a, b) = (paint(&dark, "Plex", "com.beb.plxnative"), paint(&light, "Plex", "com.beb.plxnative"));
        assert_ne!(a, b);
        assert_eq!(at(&b, 0, 0)[..3], light.panel);
        // every pixel of the light card is between its panel and its text: nothing of the dark one's colours
        let (lo, hi) = (light.text, light.panel);
        assert!(b.chunks_exact(4).all(|p| (0..3).all(|k| p[k] >= lo[k].min(hi[k]) && p[k] <= lo[k].max(hi[k]))));
    }

    #[test]
    fn an_app_without_a_title_is_named_by_its_id() {
        assert_eq!(label("", "com.apple.appletv"), "com.apple.appletv");
        assert_eq!(label("  ", "netflix"), "netflix");
        assert_eq!(label(" Marquee ", "com.beb.plxnative"), "Marquee");
    }

    #[test]
    fn a_long_title_is_cut_to_the_width_left_for_it() {
        let p = Palette::default();
        let px = paint(&p, &"Wide ".repeat(60), "x");
        let y = TILE_TOP + TILE + TITLE_GAP;
        let row = Rect::new(0, y, PICTURE_WIDTH as i32, 30);
        let lit: Vec<i32> = (row.x..row.right()).filter(|x| (row.y..row.bottom()).any(|yy| at(&px, *x, yy)[..3] != p.panel)).collect();
        let (first, last) = (*lit.first().unwrap(), *lit.last().unwrap());
        assert!(first >= (PICTURE_WIDTH as i32 - TITLE_MAX) / 2 - 2 && last <= (PICTURE_WIDTH as i32 + TITLE_MAX) / 2 + 2);
    }
}
