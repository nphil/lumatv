//! The status bar's pill is an item of the registry like the volume bar: centred along the top of the screen until the owner moves it, held to the
//! bar's row, and read from `layout.overlays.statusbar` as the other items are. Its glass can also span the whole width of the bar's row
//! (`full`), with the buttons it holds where the placement puts them inside it.

use lumatv_placement::{Anchor, Frame, Placement, Rect, Registry, Size, User, ZoneKind};
use serde_json::json;

const PILL: Size = Size::new(450, 60);

fn rect(user: &User) -> (i32, i32, i32, i32) {
    let r = Registry::shipped().rect_of("statusbar", user, &Frame::default(), PILL).expect("the status bar is an item");
    (r.x, r.y, r.w, r.h)
}

fn placed(anchor: &str, dx: f64) -> User {
    Registry::shipped().user_from_json(&json!({"statusbar": {"anchor": anchor, "dx": dx, "dy": 0}}))
}

fn full(anchor: &str, dx: f64) -> User {
    Registry::shipped().user_from_json(&json!({"statusbar": {"anchor": anchor, "dx": dx, "dy": 0, "full": true}}))
}

fn glass(user: &User, f: &Frame, size: Size) -> Rect {
    Registry::shipped().glass_of("statusbar", user, f, size).expect("the status bar is an item")
}

const PLACES: [(&str, f64); 4] = [("top-left", 0.0), ("top", 0.0), ("top-right", 0.0), ("top", 0.17)];

#[test]
fn full_width_spreads_the_glass_over_the_bars_row_and_the_buttons_stay_where_the_placement_puts_them() {
    let f = Frame::default();
    for (anchor, dx) in PLACES {
        let (off, on) = (placed(anchor, dx), full(anchor, dx));
        assert_eq!(rect(&on), rect(&off), "{anchor} {dx}: the group does not move when the glass spreads");
        let g = glass(&on, &f, PILL);
        assert_eq!((g.x, g.right(), g.y, g.h), (f.safe_x, 1920 - f.safe_x, f.safe_y, 60), "{anchor} {dx}: the safe area's width, in the bar's row");
        let r = Registry::shipped().rect_of("statusbar", &on, &f, PILL).unwrap();
        assert!(g.x <= r.x && r.right() <= g.right(), "{anchor} {dx}: the group is inside the glass");
        let own = glass(&off, &f, PILL);
        assert_eq!(own, r, "{anchor} {dx}: not spread, the glass is the group's own rectangle");
    }
    // the same at 200 % text, where the pill is as tall as the bar's row grew, and with the margins Home sends
    let layout = json!({"safe": {"x": 100, "y": 40}, "statusBar": {"height": 120, "gap": 16}});
    let f = Frame::from_json(&layout, None);
    let g = glass(&full("top-right", 0.0), &f, Size::new(700, 120));
    assert_eq!((g.x, g.right(), g.y, g.h), (100, 1820, 40, 120));
}

#[test]
fn the_ticker_is_on_unless_the_file_says_it_is_off_and_only_the_status_bar_takes_it_off() {
    let reg = Registry::shipped();
    let off = json!({"statusbar": {"anchor": "top-right", "dx": 0.0, "dy": 0.0, "full": true, "ticker": false}});
    let user = reg.user_from_json(&off);
    assert!(!reg.placement_of("statusbar", &user).unwrap().placement.ticker);
    assert_eq!(user.to_json(), off, "off is written");
    assert!(reg.placement_of("statusbar", &full("top-right", 0.0)).unwrap().placement.ticker, "a file of an earlier build has it on");
    assert!(full("top-right", 0.0).to_json()["statusbar"].get("ticker").is_none(), "on is not written");
    assert_eq!(Registry::shipped().words("statusbar", &user), "Top right, full width, no ticker");
    let junk = reg.user_from_json(&json!({"popup": {"anchor": "top-left", "dx": 0, "dy": 0, "ticker": false}, "statusbar": {"anchor": "top", "dx": 0, "dy": 0, "ticker": "no"}}));
    assert!(junk.is_empty(), "{:?}", junk.to_json());
}

#[test]
fn full_is_stored_beside_the_anchor_and_the_offsets_and_only_the_status_bar_takes_it() {
    let reg = Registry::shipped();
    let written = full("top-right", 0.0).to_json();
    assert_eq!(written, json!({"statusbar": {"anchor": "top-right", "dx": 0.0, "dy": 0.0, "full": true}}));
    assert_eq!(reg.user_from_json(&written), full("top-right", 0.0), "what is written is read back");
    // off is not written, so a file of an earlier build reads as it did
    assert_eq!(placed("top-right", 0.0).to_json(), json!({"statusbar": {"anchor": "top-right", "dx": 0.0, "dy": 0.0}}));
    // the default place with the glass spread is the owner's, and it is the default again with it off
    let spread = full("top", 0.0);
    assert!(spread.has("statusbar") && reg_custom(&spread));
    assert!(reg.with_placement(&spread, "statusbar", Placement::new(Anchor::Top, 0.0, 0.0)).is_empty());
    // not a boolean, or an item with no glass to spread: not taken
    let junk = reg.user_from_json(&json!({
        "statusbar": {"anchor": "top", "dx": 0, "dy": 0, "full": "yes"},
        "popup": {"anchor": "top-left", "dx": 0, "dy": 0, "full": true},
        "volume": {"anchor": "left", "dx": 0, "dy": 0, "full": true},
    }));
    assert!(junk.is_empty(), "{:?}", junk.to_json());
    let d = reg.describe();
    let stretches: Vec<(&str, bool)> = d["items"].as_array().unwrap().iter().map(|i| (i["id"].as_str().unwrap(), i["stretch"].as_bool().unwrap())).collect();
    assert_eq!(stretches.iter().filter(|(_, s)| *s).collect::<Vec<_>>(), [&("statusbar", true)]);
}

#[test]
fn the_words_say_where_the_buttons_are_and_that_the_glass_is_full() {
    let words = |u: &User| Registry::shipped().words("statusbar", u);
    assert_eq!(words(&full("top", 0.0)), "Top, full width");
    assert_eq!(words(&full("top-left", 0.0)), "Top left, full width");
    assert_eq!(words(&full("top", 0.1)), "Top, right third, moved, full width");
    assert_eq!(words(&placed("top-left", 0.0)), "Top left");
    assert_eq!(words(&User::default()), "Top");
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
