//! The model, one rule a test: anchors and offsets in fractions of the room an item has, the zones, the defaults (where every item was before the
//! owner could place it), stepping with the arrows, the volume bar's edges, stacking, overlaps, the mute mark's drift.

use lumatv_placement::model::{GAP, STACK, TOAST_FOOT};
use lumatv_placement::motion::{STOPS, axis, pixels};
use lumatv_placement::above_subtitles;
use lumatv_placement::{
    Anchor, Dir, Frame, Item, Layout, Placement, Point, Press, Rect, Registry, Size, Sizes, User,
    ZoneKind, column_room,
};
use serde_json::{Value, json};

fn reg() -> &'static Registry {
    Registry::shipped()
}

/// Home's own numbers, as layout.json ships them.
fn home() -> Value {
    json!({"safe": {"x": 64, "y": 54}, "statusBar": {"height": 60, "gap": 20}})
}

fn frame() -> Frame {
    Frame::from_json(&home(), Some(&json!(810)))
}

fn sizes() -> Sizes {
    let mut s = Sizes::new();
    for e in [Anchor::Left, Anchor::Right] {
        s.set_edge("volume", e, Size::new(96, 440));
    }
    s.set_edge("volume", Anchor::Bottom, Size::new(440, 60));
    s.set("mute", Size::new(64, 64))
        .set("popup", Size::new(560, 469))
        .set("cards", Size::new(560, 165))
        .set("toast", Size::new(560, 98));
    s
}

fn user(v: Value) -> User {
    reg().user_from_json(&v)
}

fn at(anchor: Anchor, dx: f64, dy: f64) -> Placement {
    Placement::new(anchor, dx, dy)
}

fn rect_of(id: &str, u: &User, w: i32, h: i32) -> Rect {
    reg().rect_of(id, u, &frame(), Size::new(w, h)).unwrap()
}

fn placed(id: &str, p: Placement) -> User {
    reg().with_placement(&User::default(), id, p)
}

fn all(u: &User, over_app: bool) -> Layout {
    reg().layout_all(u, &frame(), &sizes(), None, over_app)
}

fn step(id: &str, p: &Placement, dir: Dir, press: Press) -> lumatv_placement::Step {
    reg().step(id, p, dir, &sizes(), &frame(), press).unwrap()
}

fn tap() -> Press {
    Press::default()
}

fn held(run: u32) -> Press {
    Press { repeat: true, run }
}

fn near(a: f64, b: f64, d: f64) -> bool {
    (a - b).abs() <= d
}

#[test]
fn the_frame_is_homes_own_with_fallbacks_when_home_has_not_said() {
    let want = Frame {
        safe_x: 64,
        safe_y: 54,
        bar: 60,
        top: 134,
        limit: 810,
    };
    assert_eq!(frame(), want);
    assert_eq!(Frame::from_json(&Value::Null, None), want);
    assert_eq!(Frame::default(), want);
    let wide = Frame::from_json(
        &json!({"safe": {"x": 96, "y": 54}, "statusBar": {"height": 60, "gap": 20}}),
        Some(&json!(800)),
    );
    assert_eq!(
        wide,
        Frame {
            safe_x: 96,
            safe_y: 54,
            bar: 60,
            top: 134,
            limit: 800
        }
    );
    // a value of the wrong kind or range is not taken
    assert_eq!(
        Frame::from_json(
            &json!({"safe": {"x": "wide"}, "statusBar": {"height": -3}}),
            Some(&json!(5))
        )
        .safe_x,
        64
    );
    assert_eq!(Frame::from_json(&home(), Some(&json!(5))).limit, 810);
    assert_eq!(
        Frame::from_json(&json!({"statusBar": {"height": -3}}), None).bar,
        60
    );
}

#[test]
fn the_zones_follow_homes_content_area() {
    let (f, r) = (frame(), reg());
    assert_eq!(r.zone_of("popup", &f), Rect::new(64, 134, 1792, 676));
    assert_eq!(r.zone_of("cards", &f), r.zone_of("popup", &f));
    assert_eq!(r.zone_of("mute", &f), Rect::new(64, 134, 1792, 676));
    assert_eq!(r.zone_of("volume", &f), Rect::new(64, 134, 1792, 914));
    assert_eq!(r.zone_of("toast", &f), Rect::new(24, 24, 1872, 1002));
    // every zone but the toasts' ends where Home's grid does on the right and on the left (LayoutGrid.area)
    for id in ["popup", "cards", "mute", "volume"] {
        let z = r.zone_of(id, &f);
        assert_eq!((z.x, z.right()), (64, 1920 - 64), "{id}");
    }
    // and none lets an overlay card into the bottom quarter but the toasts, which rest at the foot of the screen with 5 % of the height
    // (54 px) of air under them
    for id in ["popup", "cards", "mute"] {
        assert!(r.zone_of(id, &f).bottom() <= 810, "{id}");
    }
    assert_eq!(r.zone_of("toast", &f).bottom(), 1080 - TOAST_FOOT);
    assert_eq!(ZoneKind::Content.name(), "content");
}

#[test]
fn defaults_are_where_the_items_were_drawn_before() {
    let e = User::default();
    // the camera window and the cards: 560 wide at the top right below the status bar's row, ending where Home's right edge does
    assert_eq!(
        rect_of("popup", &e, 560, 469),
        Rect::new(1296, 134, 560, 469)
    );
    assert_eq!(
        rect_of("cards", &e, 560, 165),
        Rect::new(1296, 134, 560, 165)
    );
    assert_eq!(
        rect_of("popup", &e, 560, 469).right(),
        1920 - frame().safe_x
    );
    // the toasts: centred at the foot of the screen, 54 px (5 %) above the bottom edge, 560 wide
    assert_eq!(rect_of("toast", &e, 560, 98), Rect::new(680, 928, 560, 98));
    // the volume bar: centred, 64 above the bottom edge, 440 by 60; over an app 120 higher
    assert_eq!(rect_of("volume", &e, 440, 60), Rect::new(740, 988, 440, 60));
    assert_eq!(reg().lift_over("volume", &e, true), 60);
    assert_eq!(reg().lift_over("volume", &e, false), 0);
    assert_eq!(1048 - 60, 1080 - 92); // over an app the bar's foot is 92 px above the bottom edge, half of what it was
    // the mute mark: the first of its four places over an app is the left corner, 96 in and 134 down
    assert_eq!(rect_of("mute", &e, 64, 64), Rect::new(96, 134, 64, 64));
    assert!(reg().is_auto("mute", &e));
    assert_eq!(reg().words("mute", &e), "Corners");
}

#[test]
fn every_item_has_a_valid_default_inside_its_zone_and_no_two_defaults_are_in_each_others_way() {
    let l = all(&User::default(), true);
    for it in reg().items() {
        assert!(reg().valid(it, &it.default), "{}", it.id);
        if it.id == "volume" {
            continue; // over an app it is lifted out of its zone by design
        }
        let (r, z) = (l.rect(it.id).unwrap(), reg().zone_of(it.id, &frame()));
        assert!(
            r.x >= z.x && r.right() <= z.right() && r.y >= z.y && r.bottom() <= z.bottom(),
            "{}",
            it.id
        );
    }
    for it in reg().items() {
        assert_eq!(
            reg().blocker(&l, it.id, &l.rect(it.id).unwrap(), None),
            None,
            "{}",
            it.id
        );
    }
    // the defaults as a file are nothing: the owner has placed nothing
    let d = |id: &str| reg().item(id).unwrap().default;
    assert!(user(json!({"volume": d("volume"), "popup": d("popup")})).is_empty());
}

#[test]
fn a_placement_is_an_anchor_and_fractions_of_the_room_left() {
    let (f, z) = (frame(), reg().zone_of("popup", &frame()));
    let corners = [
        (Anchor::TopLeft, z.x, z.y),
        (Anchor::TopRight, z.right(), z.y),
        (Anchor::BottomLeft, z.x, z.bottom()),
        (Anchor::BottomRight, z.right(), z.bottom()),
    ];
    for (a, cx, cy) in corners {
        for (w, h) in [(560, 455), (560, 300), (320, 120), (900, 676)] {
            let r = reg()
                .rect_of(
                    "popup",
                    &placed("popup", at(a, 0.0, 0.0)),
                    &f,
                    Size::new(w, h),
                )
                .unwrap();
            let name = a.name();
            assert_eq!(
                if name.ends_with("right") {
                    r.right()
                } else {
                    r.x
                },
                cx,
                "{name} {w}x{h}"
            );
            assert_eq!(
                if name.starts_with("bottom") {
                    r.bottom()
                } else {
                    r.y
                },
                cy,
                "{name} {w}x{h}"
            );
        }
    }
    let mid = rect_of(
        "popup",
        &placed("popup", at(Anchor::Center, 0.0, 0.0)),
        560,
        400,
    );
    assert!(near(
        f64::from(mid.x + 280),
        f64::from(z.x) + f64::from(z.w) / 2.0,
        1.0
    ));
    assert!(near(
        f64::from(mid.y + 200),
        f64::from(z.y) + f64::from(z.h) / 2.0,
        1.0
    ));
    // an offset is a fraction of the room: half of it from the left corner is the middle, and it stops at the zone's edge
    let half = rect_of(
        "popup",
        &placed("popup", at(Anchor::TopLeft, 0.5, 0.0)),
        560,
        400,
    );
    assert_eq!(
        half.x,
        z.x + lumatv_placement::model::round(0.5 * f64::from(z.w - 560))
    );
    assert_eq!(
        rect_of(
            "popup",
            &placed("popup", at(Anchor::TopRight, 0.4, 0.0)),
            560,
            400
        )
        .right(),
        z.right()
    );
    // an item larger than its zone sits at the zone's start and does not go negative
    assert_eq!(
        rect_of(
            "popup",
            &placed("popup", at(Anchor::BottomRight, 0.0, 0.0)),
            560,
            900
        )
        .y,
        z.y
    );
    // text at 200 %: the same corner, a larger card, still flush
    assert_eq!(rect_of("popup", &User::default(), 560, 640).right(), 1856);
}

#[test]
fn what_the_owner_placed_is_read_when_it_is_a_whole_valid_placement_of_an_item_that_exists() {
    let dflt = reg().placement_of("popup", &User::default()).unwrap();
    assert!(!dflt.custom);
    let bad = [
        json!({"anchor": "north", "dx": 0, "dy": 0}),
        json!({"anchor": "top", "dx": "a", "dy": 0}),
        json!({"anchor": "top", "dx": 0}),
        json!({"anchor": "top", "dx": 9, "dy": 0}),
        Value::Null,
        json!(5),
        json!("top"),
    ];
    for b in bad {
        assert_eq!(
            reg().placement_of("popup", &user(json!({"popup": b.clone()}))),
            Some(dflt),
            "{b}"
        );
    }
    let mine = reg()
        .placement_of(
            "popup",
            &user(json!({"popup": {"anchor": "bottom-left", "dx": 0.02, "dy": -0.01}})),
        )
        .unwrap();
    assert_eq!(
        (mine.placement, mine.custom),
        (at(Anchor::BottomLeft, 0.02, -0.01), true)
    );
    assert_eq!(reg().placement_of("nothing", &User::default()), None);
    assert_eq!(reg().user_from_json(&json!("not a map")), User::default());
    // the volume bar only lives on its edges, and an offset across the edge is dropped to nothing
    assert!(
        !reg()
            .placement_of(
                "volume",
                &user(json!({"volume": {"anchor": "top", "dx": 0, "dy": 0}}))
            )
            .unwrap()
            .custom
    );
    let b = reg()
        .placement_of(
            "volume",
            &user(json!({"volume": {"anchor": "bottom", "dx": 0.2, "dy": 0.3}})),
        )
        .unwrap();
    assert_eq!(
        (b.placement, b.custom),
        (at(Anchor::Bottom, 0.2, 0.0), true)
    );
    let l = reg()
        .placement_of(
            "volume",
            &user(json!({"volume": {"anchor": "left", "dx": 0.2, "dy": 0.3}})),
        )
        .unwrap();
    assert_eq!((l.placement, l.custom), (at(Anchor::Left, 0.0, 0.3), true));
}

#[test]
fn the_file_keeps_only_what_differs_from_the_default_whole_with_four_decimals() {
    let f = user(json!({
        "popup": {"anchor": "top-right", "dx": 0, "dy": 0},
        "toast": {"anchor": "bottom", "dx": 0.123456, "dy": 0},
        "ghost": {"anchor": "top", "dx": 0, "dy": 0},
        "cards": {"anchor": "x", "dx": 0, "dy": 0}
    }));
    assert_eq!(
        f.to_json(),
        json!({"toast": {"anchor": "bottom", "dx": 0.1235, "dy": 0.0}})
    );
    let w = reg().with_placement(&f, "mute", at(Anchor::TopRight, 0.0, 0.0));
    assert_eq!(
        w.to_json().as_object().unwrap().keys().collect::<Vec<_>>(),
        ["mute", "toast"]
    );
    // put back to the default, it leaves the file
    assert_eq!(
        reg().with_placement(&w, "mute", reg().item("mute").unwrap().default),
        f
    );
    assert_eq!(
        w.without("toast").to_json(),
        json!({"mute": {"anchor": "top-right", "dx": 0.0, "dy": 0.0}})
    );
    assert_eq!((w.customised(), User::default().customised()), (2, 0));
    // an invalid one is not written: the volume bar has no top edge, and nothing is a toolkit's own id
    assert_eq!(
        reg().with_placement(&f, "volume", at(Anchor::Top, 0.0, 0.0)),
        f
    );
    assert_eq!(
        reg().with_placement(&f, "nothing", at(Anchor::Top, 0.0, 0.0)),
        f
    );
    assert_eq!(
        reg().with_placement(&f, "popup", at(Anchor::Top, 2.0, 0.0)),
        f
    );
}

#[test]
fn fractions_become_the_nearest_anchor_and_the_small_offset_from_it() {
    let it = reg().item("popup").unwrap();
    assert_eq!(
        reg().from_fractions(it, 1.0, 0.0),
        at(Anchor::TopRight, 0.0, 0.0)
    );
    assert_eq!(
        reg().from_fractions(it, 0.97, 0.02),
        at(Anchor::TopRight, -0.03, 0.02)
    );
    assert_eq!(
        reg().from_fractions(it, 0.5, 0.5),
        at(Anchor::Center, 0.0, 0.0)
    );
    assert_eq!(reg().from_fractions(it, 1.0 / 3.0, 0.0).anchor, Anchor::Top);
    let v = reg().item("volume").unwrap();
    assert_eq!(
        reg().from_fractions(v, 0.1, 1.0),
        at(Anchor::Bottom, -0.4, 0.0)
    );
    assert_eq!(
        reg().from_fractions(v, 0.0, 0.8),
        at(Anchor::Left, 0.0, 0.3)
    );
    // the round trip: a placement read back is where it was put
    for (fx, fy) in [
        (0.0, 0.0),
        (1.0, 1.0),
        (0.37, 0.81),
        (0.5, 0.02),
        (1.0 / 3.0, 2.0 / 3.0),
    ] {
        let (px, py) = reg().from_fractions(it, fx, fy).fractions();
        assert!(near(px, fx, 0.0001) && near(py, fy, 0.0001), "{fx} {fy}");
    }
}

#[test]
fn an_arrow_moves_an_item_a_fine_step_a_tap_stops_on_the_next_stop_and_a_held_key_glides_over_them()
{
    let p = reg()
        .placement_of("popup", &User::default())
        .unwrap()
        .placement; // top right
    assert!(!step("popup", &p, Dir::Right, tap()).moved); // against the edge
    assert!(!step("popup", &p, Dir::Up, tap()).moved);
    let left = step("popup", &p, Dir::Left, tap());
    assert!(left.moved);
    let z = reg().zone_of("popup", &frame());
    assert!(near(
        (1.0 - left.placement.fractions().0) * f64::from(z.w - 560),
        12.0,
        0.6
    )); // one step of 12 px
    // taps from the right edge stop on the 2/3 stop and then on the middle, even where a step would have gone past
    let (mut q, mut seen) = (p, Vec::new());
    for _ in 0..400 {
        if q.fractions().0 <= 0.45 {
            break;
        }
        q = step("popup", &q, Dir::Left, tap()).placement;
        let x = q.fractions().0;
        if STOPS.iter().any(|s| near(x, *s, 0.0006)) {
            seen.push((x * 100.0).round() / 100.0);
        }
    }
    assert!(seen.contains(&0.67) && seen.contains(&0.5), "{seen:?}");
    // a held key does not stop on the way: it reaches the far edge flush
    let mut h = p;
    for i in 0..500 {
        h = step("popup", &h, Dir::Left, held(i)).placement;
    }
    assert_eq!(rect_of("popup", &placed("popup", h), 560, 469).x, 64);
    // an item as wide as its zone has no room to go sideways
    let mut wide = Sizes::new();
    wide.set("popup", Size::new(1792, 469));
    assert!(
        !reg()
            .step("popup", &p, Dir::Left, &wide, &frame(), tap())
            .unwrap()
            .moved
    );
    // held longer, the step grows
    assert_eq!((pixels(0), pixels(6), pixels(13)), (12.0, 36.0, 72.0));
    assert_eq!(axis(0.5, 1.0, 0.0, 12.0, false), 0.5);
}

#[test]
fn down_and_up_move_in_the_room_the_item_has_left_and_never_into_the_bottom_quarter() {
    let p = reg()
        .placement_of("popup", &User::default())
        .unwrap()
        .placement;
    let mut d = step("popup", &p, Dir::Down, tap());
    assert!(d.moved);
    assert!(rect_of("popup", &placed("popup", d.placement), 560, 469).y > 134);
    for i in 0..60 {
        d = step("popup", &d.placement, Dir::Down, held(i));
    }
    assert_eq!(
        rect_of("popup", &placed("popup", d.placement), 560, 469).bottom(),
        810
    );
}

#[test]
fn the_volume_bar_lives_on_three_edges_slides_along_them_turns_a_corner_and_flips_from_side_to_side()
 {
    let v = |p: &Placement, dir: Dir, press: Press| step("volume", p, dir, press);
    let p = reg()
        .placement_of("volume", &User::default())
        .unwrap()
        .placement; // bottom, centred
    assert_eq!(p.anchor, Anchor::Bottom);
    assert!(!v(&p, Dir::Up, tap()).moved && !v(&p, Dir::Down, tap()).moved);
    let mut r = v(&p, Dir::Right, tap());
    assert!(r.moved && r.placement.along() > 0.5);
    // along the bottom to the right end, flush: held keys stop there
    for i in 0..400 {
        r = v(&r.placement, Dir::Right, held(i));
    }
    assert_eq!(
        (r.placement.anchor, r.placement.along()),
        (Anchor::Bottom, 1.0)
    );
    assert_eq!(
        rect_of("volume", &placed("volume", r.placement), 440, 60).right(),
        1856
    );
    // a held key does not turn the corner; the next press does: onto the right edge, at its lower end
    assert!(!v(&r.placement, Dir::Right, held(50)).moved);
    let turned = v(&r.placement, Dir::Right, tap());
    assert!(turned.moved);
    assert_eq!(
        (turned.placement.anchor, turned.placement.along()),
        (Anchor::Right, 1.0)
    );
    assert_eq!(
        rect_of("volume", &placed("volume", turned.placement), 96, 440),
        Rect::new(1856 - 96, 1048 - 440, 96, 440)
    );
    // up the right edge to the top and no further
    let mut t = turned;
    for i in 0..300 {
        t = v(&t.placement, Dir::Up, held(i));
    }
    assert_eq!(t.placement.along(), 0.0);
    assert!(!v(&t.placement, Dir::Up, tap()).moved);
    assert_eq!(
        rect_of("volume", &placed("volume", t.placement), 96, 440).y,
        134
    );
    // the left key on a side edge flips it to the other side at the same height
    let flipped = v(&t.placement, Dir::Left, tap());
    assert_eq!(
        (flipped.placement.anchor, flipped.placement.along()),
        (Anchor::Left, 0.0)
    );
    assert!(!v(&flipped.placement, Dir::Left, tap()).moved);
    // down the left edge to its foot, then one press turns onto the bottom edge at its left end
    let mut l = flipped;
    for i in 0..300 {
        l = v(&l.placement, Dir::Down, held(i));
    }
    assert_eq!(l.placement.along(), 1.0);
    let b = v(&l.placement, Dir::Down, tap());
    assert_eq!(
        (b.placement.anchor, b.placement.along()),
        (Anchor::Bottom, 0.0)
    );
    assert_eq!(
        rect_of("volume", &placed("volume", b.placement), 440, 60).x,
        64
    );
}

#[test]
fn a_vertical_bar_is_as_tall_as_its_own_size_says_and_the_room_is_the_edges() {
    let left = rect_of(
        "volume",
        &placed("volume", at(Anchor::Left, 0.0, 0.0)),
        96,
        440,
    );
    assert_eq!(
        left,
        Rect::new(
            64,
            134 + lumatv_placement::model::round(0.5 * f64::from(914 - 440)),
            96,
            440
        )
    );
    let right = rect_of(
        "volume",
        &placed("volume", at(Anchor::Right, 0.0, -0.5)),
        96,
        440,
    );
    assert_eq!(right, Rect::new(1856 - 96, 134, 96, 440));
    assert_eq!(
        lumatv_placement::Placement::on_edge(Anchor::Bottom, 1.0 / 3.0),
        at(Anchor::Bottom, -0.1667, 0.0)
    );
    // only the bottom edge rises over an app
    assert_eq!(
        reg().lift_over(
            "volume",
            &placed("volume", at(Anchor::Left, 0.0, 0.0)),
            true
        ),
        0
    );
    assert_eq!(
        reg().lift_over(
            "volume",
            &placed("volume", at(Anchor::Bottom, 0.1, 0.0)),
            true
        ),
        60
    );
}

#[test]
fn the_cards_go_under_the_camera_window_when_they_share_its_corner_above_it_when_they_grow_upwards_and_stand_alone_elsewhere()
 {
    let l = all(&User::default(), false);
    assert_eq!(l.rect("popup"), Some(Rect::new(1296, 134, 560, 469)));
    assert_eq!(
        l.rect("cards"),
        Some(Rect::new(1296, 134 + 469 + 16, 560, 165))
    ); // under it, 16 apart
    assert!(l.rect("cards").unwrap().bottom() <= 810);
    // low in the zone they grow upwards: above the window when it is at the bottom too
    let low = at(Anchor::BottomRight, 0.0, 0.0);
    let l = all(
        &reg().with_placement(&placed("popup", low), "cards", low),
        false,
    );
    assert_eq!(l.rect("popup").unwrap().bottom(), 810);
    assert_eq!(
        l.rect("cards").unwrap().bottom(),
        l.rect("popup").unwrap().y - 16
    );
    // in different corners they do not touch each other
    let l = all(&placed("cards", at(Anchor::BottomLeft, 0.0, 0.0)), false);
    let c = l.rect("cards").unwrap();
    assert_eq!((c.x, c.bottom()), (64, 810));
    assert!(!c.meets(&l.rect("popup").unwrap(), 0));
    assert!(at(Anchor::TopRight, 0.0, 0.0).grows_down() && !low.grows_down());
    // the room a column of cards has: the whole zone alone, what is left beside the window
    let z = reg().zone_of("cards", &frame());
    assert_eq!(column_room(&z, None, 16, true), 676);
    assert_eq!(
        column_room(&z, Some(&Rect::new(1296, 134, 560, 469)), 16, true),
        810 - (134 + 469 + 16)
    );
    assert_eq!(
        column_room(&z, Some(&Rect::new(1296, 600, 560, 210)), 16, false),
        600 - 16 - 134
    );
}

#[test]
fn an_item_cannot_be_put_where_another_is_and_the_stacked_pair_are_not_in_each_others_way() {
    let l = all(&User::default(), true);
    let who = |id: &str, r: Rect| reg().blocker(&l, id, &r, None);
    assert_eq!(who("volume", Rect::new(1300, 140, 96, 440)), Some("popup"));
    assert_eq!(
        who("volume", Rect::new(1856 - 96, 700, 96, 300)),
        Some("cards")
    ); // the cards sit under the window
    assert_eq!(who("volume", Rect::new(64, 300, 96, 440)), None);
    assert_eq!(who("cards", l.rect("popup").unwrap()), None);
    assert_eq!(who("popup", l.rect("cards").unwrap()), None);
    // a gap of 8 px is kept between two items
    let r = l.rect("popup").unwrap();
    assert_eq!(
        who("mute", Rect::new(r.x - 64 - 4, r.y, 64, 64)),
        Some("popup")
    );
    assert_eq!(who("mute", Rect::new(r.x - 64 - 9, r.y, 64, 64)), None);
    assert_eq!(GAP, 8);
    // the toasts at the foot are clear of the camera window; over Home the bar is at the same foot, so they stack above it
    assert_eq!(who("toast", l.rect("toast").unwrap()), None);
    let home = all(&User::default(), false);
    let (bar, toast) = (home.rect("volume").unwrap(), home.rect("toast").unwrap());
    assert!(toast.bottom() + STACK <= bar.y && !toast.meets(&bar, 0), "{toast:?} over {bar:?}");
    // the layout shows them where they rest; while an app plays the surface raises them 16 above the film's subtitles (905)
    assert_eq!(l.rect("toast").unwrap(), Rect::new(680, 928, 560, 98));
    assert_eq!(above_subtitles(Rect::new(680, 928, 560, 98), STACK), Rect::new(680, 905 - 16 - 98, 560, 98));
    assert_eq!(above_subtitles(Rect::new(1336, 24, 560, 98), STACK), Rect::new(1336, 24, 560, 98), "a rectangle clear of the band stays");
    // the volume bar on the right edge is in the way of the default column: the owner has to move that first
    let right = rect_of(
        "volume",
        &placed("volume", at(Anchor::Right, 0.0, 0.0)),
        96,
        440,
    );
    assert!(who("volume", right).is_some());
}

#[test]
fn a_move_may_never_make_a_new_overlap_but_it_can_always_leave_an_old_one() {
    // at 200 % text a toast at the top right is taller than the room above the camera window: it is on it from the start
    let mut tall = sizes();
    tall.set("toast", Size::new(560, 190));
    let user = placed("toast", at(Anchor::TopRight, 0.0, 0.0));
    let l = reg().layout_all(&user, &frame(), &tall, None, false);
    let popup = l.rect("popup").unwrap();
    assert!(l.rect("toast").unwrap().meets(&popup, 8));
    // moving the camera window while the toast covers it is refused by the plain rule, and allowed when the move leaves that overlap as it was
    let next = Rect::new(popup.x - 12, popup.y, 560, 469);
    assert_eq!(reg().blocker(&l, "popup", &next, None), Some("toast"));
    assert_eq!(reg().blocker(&l, "popup", &next, Some(&popup)), None);
    // an overlap it did not have is still refused: the bar pushed onto the cards
    let on_cards = Rect::new(1300, 640, 96, 120);
    assert_eq!(
        reg().blocker(&l, "volume", &on_cards, Some(&Rect::new(740, 928, 440, 60))),
        Some("cards")
    );
}

#[test]
fn cards_that_do_not_fit_beside_a_tall_camera_window_wait_for_it() {
    let mut tall = sizes();
    tall.set("popup", Size::new(560, 676))
        .set("cards", Size::new(560, 330));
    let l = reg().layout_all(&User::default(), &frame(), &tall, None, false);
    assert_eq!(l.rect("cards").unwrap().y, 134 + 676 + STACK);
    assert!(l.waits("cards")); // it would end at 1156, below the zone
    assert!(!all(&User::default(), false).waits("cards")); // 100 % text: it fits under the window
    // standing alone it is not waiting for anything
    let l = reg().layout_all(
        &placed("cards", at(Anchor::BottomLeft, 0.0, 0.0)),
        &frame(),
        &tall,
        None,
        false,
    );
    assert!(!l.waits("cards"));
}

#[test]
fn the_room_the_cards_have_is_the_zones_less_the_windows_only_where_they_share_its_corner() {
    let room = |u: &User, w: Option<&Rect>| {
        reg().column_room_for("cards", u, &frame(), w, 16, Size::new(560, 165))
    };
    let win = Rect::new(1296, 134, 560, 469);
    assert_eq!(room(&User::default(), Some(&win)), 810 - (134 + 469 + 16)); // top right under the window
    assert_eq!(room(&User::default(), None), 676);
    assert_eq!(
        room(&User::default(), Some(&Rect::new(64, 134, 560, 469))),
        676
    ); // the window is in the other corner
    assert_eq!(
        room(
            &placed("cards", at(Anchor::BottomRight, 0.0, 0.0)),
            Some(&win)
        ),
        676
    ); // anchored low they do not meet it
}

#[test]
fn the_mark_an_owner_placed_visits_four_places_of_a_small_square_that_leans_into_the_screen_and_keeps_off_a_card()
 {
    let mk = |p: Placement, avoid: &[Rect]| {
        reg().drift_spots(
            "mute",
            &placed("mute", p),
            &frame(),
            Size::new(64, 64),
            avoid,
        )
    };
    let pt = |x, y| Point { x, y };
    assert_eq!(
        mk(at(Anchor::TopLeft, 0.0, 0.0), &[]),
        [pt(64, 134), pt(112, 134), pt(112, 182), pt(64, 182)]
    );
    let s = mk(at(Anchor::BottomRight, 0.0, 0.0), &[]);
    assert_eq!(s[0], pt(1856 - 64, 810 - 64));
    assert_eq!((s[1].x, s[2].y), (s[0].x - 48, s[0].y - 48));
    for q in &s {
        assert!(q.x >= 64 && q.x + 64 <= 1856 && q.y >= 134 && q.y + 64 <= 810);
    }
    // all four are within a 48 px step of the chosen place, so it is still the place the owner chose
    let u = placed("mute", at(Anchor::TopRight, 0.0, 0.0));
    assert_eq!(
        reg().drift_box("mute", &u, &frame(), Size::new(64, 64)),
        Some(Rect::new(1856 - 64 - 48, 134, 64 + 48, 64 + 48))
    );
    // a spot under a card that is up is left out; with none free all stay
    assert_eq!(
        mk(
            at(Anchor::TopLeft, 0.0, 0.0),
            &[Rect::new(100, 120, 200, 40)]
        ),
        [pt(112, 182), pt(64, 182)]
    );
    assert_eq!(
        mk(
            at(Anchor::TopLeft, 0.0, 0.0),
            &[Rect::new(0, 0, 1920, 1080)]
        )
        .len(),
        4
    );
}

#[test]
fn a_pointer_drags_an_item_to_a_place_and_near_a_stop_it_lands_on_the_stop() {
    let (f, z) = (frame(), reg().zone_of("popup", &frame()));
    let p = reg()
        .placement_of("popup", &User::default())
        .unwrap()
        .placement;
    // the middle of a 560 by 469 card dropped near the top right corner lands in the corner
    let c = reg()
        .at_point(
            "popup",
            f64::from(1856 - 280 - 6),
            f64::from(134 + 234 + 5),
            &sizes(),
            &f,
            &p,
        )
        .unwrap();
    assert_eq!(c, at(Anchor::TopRight, 0.0, 0.0));
    let m = reg()
        .at_point(
            "popup",
            f64::from(z.x) + f64::from(z.w) / 2.0,
            f64::from(z.y) + f64::from(z.h) / 2.0,
            &sizes(),
            &f,
            &p,
        )
        .unwrap();
    assert_eq!(m.anchor, Anchor::Center);
    // the volume bar dropped near the right edge goes to that edge and slides to where the pointer is
    let vp = reg()
        .placement_of("volume", &User::default())
        .unwrap()
        .placement;
    let e = reg()
        .at_point("volume", 1800.0, 560.0, &sizes(), &f, &vp)
        .unwrap();
    assert_eq!(e.anchor, Anchor::Right);
    assert!(e.along() > 0.4 && e.along() < 0.7);
    assert_eq!(
        reg()
            .at_point("volume", 960.0, 1000.0, &sizes(), &f, &vp)
            .unwrap()
            .anchor,
        Anchor::Bottom
    );
}

#[test]
fn words_say_where_an_item_is_in_the_fewest_and_moved_when_it_is_not_on_a_stop() {
    let w = |id: &str, p: Option<Placement>| {
        reg().words(id, &p.map_or_else(User::default, |p| placed(id, p)))
    };
    assert_eq!(w("popup", None), "Top right");
    assert_eq!(
        w("popup", Some(at(Anchor::BottomLeft, 0.0, 0.0))),
        "Bottom left"
    );
    assert_eq!(w("popup", Some(at(Anchor::Top, 0.0, 0.0))), "Top");
    assert_eq!(w("popup", Some(at(Anchor::Center, 0.0, 0.0))), "Middle");
    assert_eq!(w("popup", Some(at(Anchor::Left, 0.0, 0.0))), "Left");
    assert_eq!(
        w("popup", Some(at(Anchor::TopRight, -0.03, 0.0))),
        "Top right, moved"
    );
    assert_eq!(
        w("popup", Some(at(Anchor::Top, -1.0 / 6.0, 0.0))),
        "Top, left third"
    );
    assert_eq!(w("volume", None), "Bottom edge");
    assert_eq!(
        w("volume", Some(at(Anchor::Left, 0.0, 0.25))),
        "Left edge, lower third"
    );
    assert_eq!(
        w("volume", Some(at(Anchor::Left, 0.0, 0.4))),
        "Left edge, bottom"
    );
    assert_eq!(
        w("volume", Some(at(Anchor::Bottom, -0.5, 0.0))),
        "Bottom edge, left"
    );
    assert_eq!(w("mute", Some(at(Anchor::TopLeft, 0.0, 0.0))), "Top left");
    assert_eq!(w("mute", None), "Corners");
}

#[test]
fn a_new_item_joins_with_one_entry_placed_stepped_and_kept_off_the_others_like_the_rest() {
    let clock = Item {
        id: "clock",
        title: "Clock",
        icon: "clock",
        zone: ZoneKind::Content,
        edges: &[],
        lift: 0,
        drift: 0,
        auto: false,
        stacks: None,
        default: at(Anchor::BottomLeft, 0.0, 0.0),
        sample: Size::new(200, 80),
    };
    let r = reg().with(clock);
    let f = frame();
    assert_eq!(
        r.rect_of("clock", &User::default(), &f, Size::new(200, 80)),
        Some(Rect::new(64, 810 - 80, 200, 80))
    );
    let l = r.layout_all(&User::default(), &f, &sizes(), None, false);
    assert_eq!(l.rect("clock"), Some(Rect::new(64, 730, 200, 80))); // its sample size, since nothing measured it
    let p = r.placement_of("clock", &User::default()).unwrap().placement;
    assert!(
        r.step("clock", &p, Dir::Right, &Sizes::new(), &f, tap())
            .unwrap()
            .moved
    );
    assert_eq!(r.words("clock", &User::default()), "Bottom left");
    assert_eq!(
        r.user_from_json(&json!({"clock": {"anchor": "top", "dx": 0, "dy": 0}}))
            .to_json(),
        json!({"clock": {"anchor": "top", "dx": 0.0, "dy": 0.0}})
    );
    // the shipped registry is not changed by it
    assert!(reg().item("clock").is_none());
}

#[test]
fn an_item_arrives_from_the_edge_it_sits_against() {
    let s = |id: &str, p: Option<Placement>| {
        reg().slide_of(id, &p.map_or_else(User::default, |p| placed(id, p)))
    };
    let slide = |x, y| lumatv_placement::Slide { x, y };
    assert_eq!(s("popup", None), slide(1, 0));
    assert_eq!(
        s("popup", Some(at(Anchor::TopLeft, 0.0, 0.0))),
        slide(-1, 0)
    );
    assert_eq!(s("popup", Some(at(Anchor::Top, 0.0, 0.0))), slide(0, -1)); // in the middle of the width: from the top or the bottom
    assert_eq!(s("popup", Some(at(Anchor::Bottom, 0.0, 0.0))), slide(0, 1));
    assert_eq!(
        s("toast", Some(at(Anchor::BottomRight, 0.0, 0.0))),
        slide(1, 0)
    );
}

#[test]
fn the_registry_describes_itself_for_a_client() {
    let d = reg().describe();
    let ids: Vec<&str> = d["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["volume", "mute", "popup", "cards", "toast"]);
    assert_eq!(d["items"][0]["edges"], json!(["left", "bottom", "right"]));
    assert_eq!(d["items"][3]["stacks"], json!("popup"));
    assert_eq!(d["items"][2]["sample"], json!({"w": 560, "h": 469}));
    assert_eq!(
        d["rules"]["subtitles"],
        json!({"top": 905, "bottom": 1010, "clear": 9})
    );
}

#[test]
fn sizes_are_read_from_json_per_item_and_per_edge() {
    let s = Sizes::from_json(
        &json!({"popup": {"w": 560.4, "h": 469}, "volume": {"left": {"w": 96, "h": 440}, "bottom": {"w": 440, "h": 60}}, "bad": {"w": -1, "h": 3}, "toast": {"w": "x", "h": 3}}),
    );
    let (popup, volume, toast) = (
        reg().item("popup").unwrap(),
        reg().item("volume").unwrap(),
        reg().item("toast").unwrap(),
    );
    assert_eq!(s.at(popup, Anchor::TopRight), Size::new(560, 469));
    assert_eq!(s.at(volume, Anchor::Left), Size::new(96, 440));
    assert_eq!(s.at(volume, Anchor::Bottom), Size::new(440, 60));
    assert_eq!(s.at(volume, Anchor::Right), volume.sample); // not measured: its sample's
    assert_eq!(s.at(toast, Anchor::TopRight), toast.sample);
}
