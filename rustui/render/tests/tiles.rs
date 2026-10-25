//! The bitmap of tiles the renderer's damage accounts are kept in.

use lumatv_render::tiles::{Ring, TILE, TileSet, visible};

const W: u32 = 1920;
const H: u32 = 1080;

fn set(areas: &[[i32; 4]]) -> TileSet {
    let mut s = TileSet::new(W, H);
    for &a in areas {
        s.add(a);
    }
    s
}

fn rects(s: &TileSet) -> Vec<[i32; 4]> {
    let mut out = Vec::new();
    s.rects(&mut out);
    out
}

#[test]
fn an_area_adds_every_tile_it_touches_and_never_past_the_panel() {
    let mut s = TileSet::new(W, H);
    s.add([17, 33, 40, 48]);
    assert_eq!(rects(&s), [[16, 32, 48, 48]]);
    s.clear();
    s.add([1900, 1070, 3000, 3000]);
    // the last tile row of 1080 px is 8 px tall, and the grid has 120 columns
    assert_eq!(rects(&s), [[1888, 1056, 1920, 1080]]);
    assert_eq!(s.pixels(), 32 * 24);
    s.clear();
    s.add([-50, -50, 1, 1]);
    assert_eq!(rects(&s), [[0, 0, 16, 16]]);
    s.clear();
    s.add([5, 5, 5, 50]);
    assert!(s.is_empty(), "an empty area touches nothing");
}

#[test]
fn an_inside_area_adds_only_the_tiles_it_covers_whole() {
    let mut s = TileSet::new(W, H);
    s.add_inside([17, 33, 80, 100]);
    // x: 32..80 whole tiles, y: 48..96
    assert_eq!(rects(&s), [[32, 48, 80, 96]]);
    s.clear();
    s.add_inside([0, 0, 1920, 1080]);
    assert_eq!(s.pixels(), 1920 * 1080, "the whole panel covers the short edge tiles too");
    assert_eq!(s.count(), 120 * 68);
    s.clear();
    s.add_inside([1, 1, 31, 31]);
    assert!(s.is_empty(), "a rectangle that holds no whole tile covers none");
}

#[test]
fn pixels_count_the_edge_tiles_clipped_to_the_panel() {
    let full = TileSet::full(W, H);
    assert_eq!(full.pixels(), 1920 * 1080);
    assert_eq!(full.count(), 8160);
    let odd = TileSet::full(100, 40);
    assert_eq!(odd.pixels(), 100 * 40);
    assert_eq!(odd.count(), 7 * 3);
}

#[test]
fn rects_are_runs_per_row_and_equal_runs_on_the_rows_below_join_them() {
    // a frame with a hole: the margins of a page of cards
    let mut s = TileSet::full(W, H);
    let mut hole = TileSet::new(W, H);
    hole.add([64, 128, 1856, 640]);
    s.subtract(&hole);
    let out = rects(&s);
    // top band, left margin, right margin, bottom band
    assert_eq!(out.len(), 4, "{out:?}");
    assert!(out.contains(&[0, 0, 1920, 128]));
    assert!(out.contains(&[0, 128, 64, 640]));
    assert!(out.contains(&[1856, 128, 1920, 640]));
    assert!(out.contains(&[0, 640, 1920, 1080]));
    let total: u64 = out.iter().map(|r| (r[2] - r[0]) as u64 * (r[3] - r[1]) as u64).sum();
    assert_eq!(total, s.pixels());
}

#[test]
fn rects_that_do_not_line_up_stay_apart_and_cover_exactly_the_tiles() {
    let s = set(&[[0, 0, 48, 16], [0, 16, 32, 32], [200, 0, 216, 64]]);
    let out = rects(&s);
    let mut check = TileSet::new(W, H);
    for r in &out {
        check.add(*r);
    }
    assert_eq!(check, s);
    assert_eq!(out.len(), 3, "{out:?}");
}

#[test]
fn union_subtract_touches_and_bounds() {
    let a = set(&[[0, 0, 32, 32]]);
    let mut b = set(&[[16, 16, 64, 64]]);
    assert!(b.touches([20, 20, 21, 21]));
    assert!(!b.touches([0, 0, 16, 16]));
    assert!(!b.touches([100, 100, 200, 200]));
    b.union(&a);
    assert_eq!(b.bounds(), Some([0, 0, 64, 64]));
    b.subtract(&a);
    assert_eq!(b.bounds(), Some([16, 16, 64, 64]));
    assert!(!b.touches([0, 0, 32, 16]));
    assert_eq!(TileSet::new(W, H).bounds(), None);
    let mut c = TileSet::new(W, H);
    c.add([1900, 1070, 1920, 1080]);
    assert_eq!(c.bounds(), Some([1888, 1056, 1920, 1080]), "the bounds stop at the panel's edge");
}

#[test]
fn union_within_takes_only_the_tiles_of_the_area() {
    let other = set(&[[0, 0, 160, 16]]);
    let mut s = TileSet::new(W, H);
    s.union_within(&other, [32, 0, 64, 16]);
    assert_eq!(rects(&s), [[32, 0, 64, 16]]);
}

#[test]
fn a_wide_panel_uses_more_than_one_word_a_row() {
    let mut s = TileSet::new(4096, 64);
    s.add([1000, 0, 3000, 16]);
    assert_eq!(rects(&s), [[992, 0, 3008, 16]]);
    s.clear();
    s.add_inside([0, 0, 4096, 64]);
    assert_eq!(s.pixels(), 4096 * 64);
    assert_eq!(s.count(), 256 * 4);
}

#[test]
fn what_is_seen_leaves_out_what_an_opaque_layer_above_covers_whole() {
    let page = [0, 0, W as i32, H as i32];
    let card = [64, 64, 640, 480];
    let layers: [(_, &[[i32; 4]]); 3] = [
        (page, &[]),                 // a wallpaper under everything
        (card, &[card]),             // an opaque card
        ([100, 100, 200, 200], &[]), // a label on the card
    ];
    let mut out = Vec::new();
    visible(W, H, &layers, &mut out);
    assert_eq!(out.len(), 3);
    // the label is on top: all of it
    assert_eq!(rects(&out[2]), [[96, 96, 208, 208]]);
    // the card is the card's tiles, no layer is over it
    assert_eq!(rects(&out[1]), [[64, 64, 640, 480]]);
    // the wallpaper is everything but the tiles inside the card
    let mut wall = TileSet::full(W, H);
    let mut hole = TileSet::new(W, H);
    hole.add_inside(card);
    wall.subtract(&hole);
    assert_eq!(out[0], wall);
    assert_eq!(out[0].pixels(), 1920 * 1080 - (640 - 64) * (480 - 64));
    // run again over the same buffers: nothing is left over from the last
    visible(W, H, &layers[..2], &mut out);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], wall);
}

#[test]
fn a_ring_owes_each_page_what_the_frames_since_its_turn_changed() {
    let mut ring = Ring::new(3, W, H);
    let mut into = TileSet::new(W, H);
    let mut damage = TileSet::new(W, H);
    // First turn: every page owes everything.
    for slot in 0..3 {
        damage.clear();
        damage.add([slot as i32 * 96, 0, slot as i32 * 96 + 16, 16]);
        ring.frame(slot, &damage, &mut into);
        assert_eq!(into.pixels(), 1920 * 1080, "page {slot} was never drawn");
    }
    // Second turn: page 0 owes what frames 1 and 2 changed, and its own.
    damage.clear();
    damage.add([288, 288, 304, 304]);
    ring.frame(0, &damage, &mut into);
    assert_eq!(into.count(), 3);
    for r in [[96, 0, 112, 16], [192, 0, 208, 16], [288, 288, 304, 304]] {
        assert!(into.touches(r), "{r:?}");
    }
    // A frame that changes nothing still owes what the other pages did since.
    damage.clear();
    ring.frame(1, &damage, &mut into);
    assert_eq!(into.count(), 2, "page 1 owes what frames 2 and 3 changed");
    assert_eq!(ring.owed_pixels(2), 256, "page 2 owes the one tile frame 3 changed");
    ring.forget();
    ring.frame(2, &damage, &mut into);
    assert_eq!(into.pixels(), 1920 * 1080);
    // a page that is not in the ring is owed everything
    ring.frame(7, &damage, &mut into);
    assert_eq!(into.pixels(), 1920 * 1080);
    assert_eq!(TILE, 16);
}
