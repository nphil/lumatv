//! The status bar's pill is an item of the registry like the volume bar: centred along the top of the screen until the owner moves it, held to the
//! bar's row, and read from `layout.overlays.statusbar` as the other items are.

use lumatv_placement::{Anchor, Frame, Placement, Registry, Size, User, ZoneKind};
use serde_json::json;

const PILL: Size = Size::new(450, 60);

fn rect(user: &User) -> (i32, i32, i32, i32) {
    let r = Registry::shipped().rect_of("statusbar", user, &Frame::default(), PILL).expect("the status bar is an item");
    (r.x, r.y, r.w, r.h)
}

fn placed(anchor: &str, dx: f64) -> User {
    Registry::shipped().user_from_json(&json!({"statusbar": {"anchor": anchor, "dx": dx, "dy": 0}}))
}

#[test]
fn the_status_bar_is_centred_along_the_top_until_the_owner_moves_it() {
    let reg = Registry::shipped();
    let item = reg.item("statusbar").expect("an item");
    assert_eq!((item.zone, item.default), (ZoneKind::Bar, Placement::new(Anchor::Top, 0.0, 0.0)));
    assert!(!reg.placement_of("statusbar", &User::default()).unwrap().custom);
    let (x, y, w, _) = rect(&User::default());
    assert_eq!((x + w / 2, y), (960, 54), "the middle of the screen, at the safe area's top");
}

#[test]
fn left_and_right_are_flush_with_the_safe_margins_and_an_offset_moves_it_along_the_bar() {
    let f = Frame::default();
    assert_eq!(rect(&placed("top-left", 0.0)).0, f.safe_x);
    assert_eq!(rect(&placed("top-right", 0.0)).0 + PILL.w, 1920 - f.safe_x);
    let room = 1920 - 2 * f.safe_x - PILL.w;
    let (x, y, ..) = rect(&placed("top", 0.25));
    assert_eq!((x, y), (f.safe_x + (0.75 * f64::from(room) + 0.5).floor() as i32, 54), "a quarter of the room to the right of the middle");
    assert!(reg_custom(&placed("top", 0.25)), "an offset is the owner's");
}

fn reg_custom(user: &User) -> bool {
    Registry::shipped().placement_of("statusbar", user).unwrap().custom
}

#[test]
fn it_stays_on_the_screen_and_in_its_row_whatever_the_file_says() {
    let f = Frame::default();
    for (anchor, dx, dy) in [("top-left", -1.0, 0.0), ("top-right", 1.0, 0.0), ("top", 3.0, 0.0), ("bottom", 0.0, 0.9), ("center", 0.4, -0.4)] {
        let user = Registry::shipped().user_from_json(&json!({"statusbar": {"anchor": anchor, "dx": dx, "dy": dy}}));
        let (x, y, w, h) = rect(&user);
        assert!(x >= f.safe_x && x + w <= 1920 - f.safe_x, "{anchor} {dx}: on the screen, inside the safe margins ({x}..{})", x + w);
        assert_eq!((y, y + h), (f.safe_y, f.safe_y + 60), "{anchor} {dy}: the bar's row is the only room it has");
    }
    // a pill taller than its row (large text) still starts at the row's top, and one wider than the room starts at its left
    let tall = Registry::shipped().rect_of("statusbar", &placed("top", 0.0), &f, Size::new(450, 120)).unwrap();
    assert_eq!(tall.y, f.safe_y);
    let wide = Registry::shipped().rect_of("statusbar", &placed("top-right", 0.0), &f, Size::new(2000, 60)).unwrap();
    assert_eq!(wide.x, f.safe_x);
}

#[test]
fn anchors_are_parsed_by_name_and_a_name_that_is_not_one_leaves_it_at_the_default() {
    assert_eq!(Registry::shipped().placement_of("statusbar", &placed("top-left", 0.0)).unwrap().placement.anchor, Anchor::TopLeft);
    assert_eq!(Registry::shipped().placement_of("statusbar", &placed("top-right", 0.0)).unwrap().placement.anchor, Anchor::TopRight);
    let junk = placed("middle-ish", 0.0);
    assert!(junk.is_empty(), "an anchor that is not one of the nine is not taken");
    assert_eq!(rect(&junk), rect(&User::default()));
    // what is written is read back
    let written = placed("top-right", 0.0).to_json();
    assert_eq!(written["statusbar"]["anchor"], "top-right");
    assert_eq!(rect(&Registry::shipped().user_from_json(&written)), rect(&placed("top-right", 0.0)));
}
