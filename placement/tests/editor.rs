//! The editor's controller, driven the way a toolkit drives it: keys and a pointer in, effects and a view out.

use lumatv_placement::{
    Anchor, Editor, Effect, Frame, Key, Placement, Rect, Registry, Size, Sizes, User,
};
use serde_json::{Value, json};

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

fn editor_on(user: Value) -> Editor {
    let reg = Registry::shipped().clone();
    let u = reg.user_from_json(&user);
    Editor::new(reg, u, Frame::default(), sizes())
}

/// Keys pressed `gap` ms apart from `t`: a gap under the held-key threshold makes them one held key.
struct Pad {
    t: u64,
}

impl Pad {
    fn new() -> Self {
        Pad { t: 10_000 }
    }

    fn press(&mut self, e: &mut Editor, keys: &[Key], gap: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        for k in keys {
            self.t += gap;
            fx.extend(e.key(*k, self.t));
        }
        fx
    }

    /// Single presses, each far enough from the last to be a tap.
    fn taps(&mut self, e: &mut Editor, keys: &[Key]) -> Vec<Effect> {
        self.press(e, keys, 1000)
    }

    /// One key held for `n` presses.
    fn hold(&mut self, e: &mut Editor, key: Key, n: usize) -> Vec<Effect> {
        let keys = vec![key; n];
        self.press(e, &keys, 40)
    }
}

fn rect(e: &Editor, id: &str, now: u64) -> Rect {
    e.view(now).items.iter().find(|i| i.id == id).unwrap().rect
}

fn saved(fx: &[Effect]) -> Value {
    match fx {
        [Effect::Save(v)] => v.clone(),
        other => panic!("expected one save, got {other:?}"),
    }
}

#[test]
fn it_opens_on_the_item_asked_for_with_nothing_in_the_hand() {
    let mut e = editor_on(json!({}));
    assert_eq!(e.view(0).selected, "volume");
    e.start(Some("popup"));
    let v = e.view(0);
    assert_eq!((v.selected, v.mode, v.focus), ("popup", "browse", "items"));
    assert_eq!(v.heading, Some("Layout of pop-ups"));
    assert_eq!(v.subject, "Camera pop-up  \u{b7}  Top right");
    assert_eq!(
        v.hint,
        "\u{2190} \u{2192} Choose  \u{b7}  OK Pick up  \u{b7}  Down Buttons  \u{b7}  Back Done"
    );
    assert_eq!(v.buttons.len(), 3);
    assert!(
        !v.buttons[0].enabled
            && !v.buttons[1].enabled
            && v.buttons[2].enabled
            && v.buttons[2].primary
    );
    e.start(Some("nothing"));
    assert_eq!(e.view(0).selected, "volume");
    assert_eq!(v.items.len(), 6);
    // every item in its own size, the volume bar as it sits over an app (60 higher), the cards under the window
    let r = |id: &str| rect(&e, id, 0);
    assert_eq!(r("volume"), Rect::new(740, 928, 440, 60));
    assert_eq!(r("popup"), Rect::new(1328, 32, 560, 469));
    assert_eq!(r("cards"), Rect::new(1328, 32 + 469 + 16, 560, 165));
    assert_eq!(v.guides.safe, Rect::new(64, 54, 1792, 972));
    assert_eq!(v.guides.status_bar, Rect::new(64, 54, 1792, 60));
    assert_eq!(v.guides.bottom_quarter, Rect::new(0, 810, 1920, 270));
}

#[test]
fn the_arrows_choose_an_item_and_the_ends_hold() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    pad.taps(&mut e, &[Key::Right, Key::Right]);
    assert_eq!(e.selected(), "popup");
    pad.taps(&mut e, &[Key::Right, Key::Right, Key::Right, Key::Right]);
    assert_eq!(e.selected(), "statusbar", "the cards, the toasts, the status bar, and the end holds");
    pad.taps(&mut e, &[Key::Left, Key::Left, Key::Up]);
    assert_eq!(e.selected(), "popup");
    for _ in 0..6 {
        pad.taps(&mut e, &[Key::Left]);
    }
    assert_eq!(e.selected(), "volume");
}

#[test]
fn an_item_is_picked_up_moved_in_fine_steps_and_put_down_where_it_is_and_only_then_saved() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("popup"));
    assert!(pad.taps(&mut e, &[Key::Ok]).is_empty());
    let v = e.view(pad.t);
    assert_eq!(
        (v.mode, v.hint, v.heading, v.buttons.len()),
        (
            "move",
            "Arrows move it  \u{b7}  OK Put down  \u{b7}  Back Put back",
            None,
            0
        )
    );
    assert!(v.items.iter().find(|i| i.id == "popup").unwrap().in_hand);
    assert!(pad.taps(&mut e, &[Key::Left]).is_empty()); // nothing is written until it is put down
    let moved = rect(&e, "popup", pad.t);
    assert_eq!((1328 - moved.x, moved.y), (12, 32)); // one fine step of 12 px
    assert_eq!(
        e.view(pad.t).held_rect,
        Some(Rect::new(1328, 32, 560, 469))
    ); // its old place stays outlined
    assert!(e.user().is_empty());
    let out = saved(&pad.taps(&mut e, &[Key::Ok]));
    let p = out["popup"].clone();
    assert_eq!(p["anchor"], json!("top-right"));
    assert!(p["dx"].as_f64().unwrap() < 0.0 && p["dy"].as_f64().unwrap() == 0.0);
    assert_eq!(e.view(pad.t).mode, "browse");
    assert!(e.view(pad.t).buttons[0].enabled); // Reset this is open now
    assert_eq!(
        e.view(pad.t)
            .items
            .iter()
            .find(|i| i.id == "popup")
            .unwrap()
            .words,
        "Top right, moved"
    );
}

#[test]
fn back_puts_an_item_back_and_writes_nothing() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("popup"));
    pad.taps(&mut e, &[Key::Ok, Key::Left, Key::Down]);
    assert_ne!(rect(&e, "popup", pad.t), Rect::new(1328, 32, 560, 469));
    assert!(pad.taps(&mut e, &[Key::Back]).is_empty());
    assert_eq!(rect(&e, "popup", pad.t), Rect::new(1328, 32, 560, 469));
    assert!(e.user().is_empty());
    assert_eq!(e.view(pad.t).mode, "browse");
    // and put down where it was picked up, nothing is saved either
    pad.taps(&mut e, &[Key::Ok]);
    assert!(pad.taps(&mut e, &[Key::Ok]).is_empty());
}

#[test]
fn a_held_key_glides_to_the_zones_edge_and_stops_flush() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("popup"));
    pad.taps(&mut e, &[Key::Ok]);
    pad.hold(&mut e, Key::Down, 200);
    // it stops flush at the foot of its zone: the bottom quarter is the subtitles'
    let r = rect(&e, "popup", pad.t);
    assert_eq!(
        (r.right(), r.bottom(), e.view(pad.t).notice.as_str()),
        (1888, 810, "")
    );
    pad.taps(&mut e, &[Key::Back]);
    // from a corner of its own, held left it ends at the zone's start, flush
    let mut e = editor_on(
        json!({"popup": {"anchor": "bottom-right", "dx": 0, "dy": 0}, "cards": {"anchor": "top-left", "dx": 0, "dy": 0}}),
    );
    e.start(Some("popup"));
    pad.taps(&mut e, &[Key::Ok]);
    pad.hold(&mut e, Key::Left, 400);
    let r = rect(&e, "popup", pad.t);
    assert_eq!((r.x, r.bottom()), (32, 810));
}

#[test]
fn an_item_is_refused_where_another_is_and_the_one_in_the_way_is_named() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("popup"));
    pad.taps(&mut e, &[Key::Ok]);
    pad.hold(&mut e, Key::Left, 300); // along the top, to the mute mark's corner
    let v = e.view(pad.t);
    let r = v.items.iter().find(|i| i.id == "popup").unwrap().rect;
    assert!(r.x >= 96 + 64 + 8, "{r:?}"); // the mark's right edge and the gap
    assert_eq!(v.notice, "Mute mark is in the way");
    assert!(v.items.iter().find(|i| i.id == "mute").unwrap().flagged);
    // the notice goes by itself
    let later = e.view(pad.t + 3000);
    assert_eq!(later.notice, "");
    assert!(!later.items.iter().find(|i| i.id == "mute").unwrap().flagged);
}

#[test]
fn the_volume_bar_turns_the_corner_on_a_fresh_press_and_a_side_edge_flips_with_the_other_key() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("volume"));
    pad.taps(&mut e, &[Key::Ok]);
    pad.hold(&mut e, Key::Left, 300); // along the bottom edge to its left end
    let r = rect(&e, "volume", pad.t);
    assert_eq!((r.x, r.w, r.h), (64, 440, 60));
    pad.taps(&mut e, &[Key::Left]); // the next press turns the corner onto the left edge, at its lower end
    let r = rect(&e, "volume", pad.t);
    assert_eq!((r.x, r.w, r.h, r.bottom()), (64, 96, 440, 1048)); // vertical, flush with Home's edge, its foot where the bottom edge's was
    pad.taps(&mut e, &[Key::Ok]);
    assert_eq!(e.user().get("volume").unwrap().anchor, Anchor::Left);
    // the opposite key flips it to the right edge at the same height, where the cards are (the camera window is above it): refused
    pad.taps(&mut e, &[Key::Ok, Key::Right]);
    assert_eq!(e.view(pad.t).notice, "Notifications is in the way");
    pad.taps(&mut e, &[Key::Back]);
    assert_eq!(e.user().get("volume").unwrap().anchor, Anchor::Left);
}

#[test]
fn reset_this_and_reset_all_put_items_back_through_the_buttons() {
    let (mut e, mut pad) = (
        editor_on(
            json!({"toast": {"anchor": "bottom-right", "dx": 0, "dy": 0}, "popup": {"anchor": "left", "dx": 0, "dy": 0}}),
        ),
        Pad::new(),
    );
    e.start(Some("popup"));
    assert_eq!(e.view(pad.t).customised, 2);
    // Down goes to the buttons, on Done; Left twice reaches Reset this
    let out = saved(&pad.taps(&mut e, &[Key::Down, Key::Left, Key::Left, Key::Ok]));
    assert_eq!(
        out,
        json!({"toast": {"anchor": "bottom-right", "dx": 0.0, "dy": 0.0}})
    );
    assert_eq!(e.view(pad.t).notice, "Back to where it was");
    assert_eq!(rect(&e, "popup", pad.t), Rect::new(1328, 32, 560, 469));
    assert!(pad.taps(&mut e, &[Key::Ok]).is_empty()); // the window is not placed any more: Reset this has nothing to do
    // Reset all
    let out = saved(&pad.taps(&mut e, &[Key::Right, Key::Ok]));
    assert_eq!(out, json!({}));
    assert_eq!(e.view(pad.t).notice, "Everything back to where it was");
    // with nothing placed both are quiet
    assert!(pad.taps(&mut e, &[Key::Ok]).is_empty());
    assert!(!e.view(pad.t).buttons[1].enabled);
}

#[test]
fn done_and_back_leave_the_editor() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    assert_eq!(pad.taps(&mut e, &[Key::Back]), [Effect::Close]);
    assert_eq!(pad.taps(&mut e, &[Key::Down, Key::Ok]), [Effect::Close]); // on Done
    // Back from the buttons goes up to the items first
    assert!(pad.taps(&mut e, &[Key::Down, Key::Back]).is_empty());
    assert_eq!(e.view(pad.t).focus, "items");
    // a pointer on a button does what OK does there
    assert_eq!(e.press_button(2, pad.t), [Effect::Close]);
}

#[test]
fn a_pointer_takes_an_item_where_it_is_pressed_follows_it_and_puts_it_down_where_it_lets_go() {
    let mut e = editor_on(json!({}));
    let r = rect(&e, "popup", 0);
    let (px, py) = (f64::from(r.x) + 100.0, f64::from(r.y) + 80.0);
    e.pointer_down("popup", px, py);
    e.pointer_move(px - 2.0, py - 2.0, true, 1); // a click is not a drag
    assert_eq!(e.view(1).mode, "browse");
    // dragged so that its middle is the middle of the zone: it lands on the middle, which is a stop
    let (tx, ty) = (960.0 + 100.0 - 280.0, 472.0 + 80.0 - 234.5);
    e.pointer_move(tx, ty, true, 2);
    assert_eq!(e.view(2).mode, "move");
    assert!(e.view(2).dragging);
    let out = saved(&e.pointer_up());
    assert_eq!(out["popup"]["anchor"], json!("center"));
    assert_eq!(e.view(3).mode, "browse");
    // a drag onto the mute mark is refused where it is made, and nothing is written on the drop
    e.pointer_down("popup", 960.0, 472.0);
    e.pointer_move(130.0, 166.0, true, 4);
    let v = e.view(4);
    assert_eq!(v.notice, "Mute mark is in the way");
    assert!(e.pointer_up().is_empty());
    assert_eq!(e.user().get("popup").unwrap().anchor, Anchor::Center);
}

#[test]
fn the_panel_goes_where_no_item_is_and_never_on_the_one_in_the_hand() {
    let mut e = editor_on(json!({}));
    for sel in ["volume", "mute", "popup", "cards", "toast"] {
        e.start(Some(sel));
        let p = e.panel_spot(760, 300);
        let panel = Rect::new(p.x, p.y, 760, 300);
        let v = e.view(0);
        let item = v.items.iter().find(|i| i.id == sel).unwrap();
        assert!(
            !panel.meets(&item.rect, 16),
            "{sel}: {panel:?} on {:?}",
            item.rect
        );
        assert!(panel.x >= 64 && panel.right() <= 1856, "{sel}: {panel:?}");
    }
}

#[test]
fn a_new_map_from_the_caller_replaces_what_the_editor_knows_and_puts_a_move_back() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("popup"));
    pad.taps(&mut e, &[Key::Ok, Key::Left]);
    let reg = Registry::shipped();
    let other = reg.with_placement(
        &User::default(),
        "popup",
        Placement::new(Anchor::BottomLeft, 0.0, 0.0),
    );
    e.set_user(other);
    assert_eq!(e.view(pad.t).mode, "browse");
    assert_eq!(rect(&e, "popup", pad.t), Rect::new(32, 810 - 469, 560, 469));
}

#[test]
fn an_editor_on_an_empty_registry_takes_every_key_without_falling_over() {
    let mut e = Editor::new(
        Registry::new(Vec::new()),
        User::default(),
        Frame::default(),
        Sizes::new(),
    );
    let mut pad = Pad::new();
    pad.taps(
        &mut e,
        &[Key::Right, Key::Left, Key::Ok, Key::Down, Key::Ok, Key::Up],
    );
    let v = e.view(pad.t);
    assert!(v.items.is_empty() && v.selected.is_empty());
    assert_eq!(pad.taps(&mut e, &[Key::Back]), [Effect::Close]);
    assert_eq!(e.panel_spot(760, 300).y, 390);
}

fn labels(e: &Editor, now: u64) -> Vec<&'static str> {
    e.view(now).buttons.iter().map(|b| b.label).collect()
}

fn glass(e: &Editor, id: &str, now: u64) -> Rect {
    e.view(now).items.iter().find(|i| i.id == id).unwrap().glass
}

#[test]
fn only_the_status_bar_has_a_full_width_switch_and_it_comes_before_the_other_buttons() {
    let mut e = editor_on(json!({}));
    for plain in ["volume", "mute", "popup", "cards", "toast"] {
        e.start(Some(plain));
        assert_eq!(labels(&e, 0), ["Reset this", "Reset all", "Done"], "{plain}");
        assert_eq!(e.button_count(), 3);
        let v = e.view(0);
        assert!(v.buttons.iter().all(|b| b.icon.is_none() && !b.selected), "{plain}");
        assert_eq!(glass(&e, plain, 0), rect(&e, plain, 0), "{plain}: its glass is its own rectangle");
    }
    e.start(Some("statusbar"));
    assert_eq!(labels(&e, 0), ["Full width", "Reset this", "Reset all", "Done"]);
    assert_eq!(e.button_count(), 4);
    let v = e.view(0);
    assert!(v.buttons[0].enabled && !v.buttons[0].primary && !v.buttons[0].selected);
    assert_eq!(v.buttons[0].icon, Some("circle"), "off: an empty mark");
    assert!(!v.buttons[1].enabled && !v.buttons[2].enabled && v.buttons[3].primary, "nothing placed: nothing to reset, and Done is the primary");
    // a place the panel does not have does nothing
    assert!(e.press_button(4, 1).is_empty());
    e.start(Some("popup"));
    assert!(e.press_button(3, 1).is_empty(), "the camera window has no fourth button");
    assert_eq!(e.press_button(2, 1), [Effect::Close]);
}

#[test]
fn the_switch_spreads_the_glass_across_the_bar_and_back_without_moving_what_it_holds() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("statusbar"));
    let held = rect(&e, "statusbar", 0);
    assert_eq!(held, Rect::new(735, 54, 450, 60), "centred along the bar");
    assert_eq!(glass(&e, "statusbar", 0), held);
    // Down goes to the buttons, on Done; Left three times reaches Full width, and OK turns it on
    let out = saved(&pad.taps(&mut e, &[Key::Down, Key::Left, Key::Left, Key::Left, Key::Ok]));
    assert_eq!(out, json!({"statusbar": {"anchor": "top", "dx": 0.0, "dy": 0.0, "full": true}}));
    let v = e.view(pad.t);
    assert_eq!(v.subject, "Status bar  \u{b7}  Top, full width");
    assert_eq!(glass(&e, "statusbar", pad.t), Rect::new(64, 54, 1792, 60), "the safe area's width in the bar's row");
    assert_eq!(rect(&e, "statusbar", pad.t), held, "the buttons stay where they were");
    assert!(v.buttons[0].selected && v.buttons[0].focused && v.buttons[0].icon == Some("circle-check"));
    assert!(v.buttons[1].enabled, "it is the owner's now: Reset this has something to put back");
    // OK again: the default place with the glass round its buttons is the default, and nothing is placed
    let out = saved(&pad.taps(&mut e, &[Key::Ok]));
    assert_eq!(out, json!({}));
    assert_eq!(glass(&e, "statusbar", pad.t), held);
    assert!(!e.view(pad.t).buttons[0].selected && !e.view(pad.t).buttons[1].enabled);
    // a pointer on the switch does what OK does there
    let out = saved(&e.press_button(0, pad.t));
    assert_eq!(out["statusbar"]["full"], json!(true));
    assert_eq!(e.view(pad.t).focus, "buttons");
}

#[test]
fn the_focus_on_the_buttons_falls_back_to_done_for_an_item_that_has_no_such_button() {
    let (mut e, mut pad) = (editor_on(json!({})), Pad::new());
    e.start(Some("statusbar"));
    pad.taps(&mut e, &[Key::Down, Key::Left, Key::Left, Key::Left]);
    assert!(e.view(pad.t).buttons[0].focused);
    pad.taps(&mut e, &[Key::Left]);
    assert!(e.view(pad.t).buttons[0].focused, "the end holds");
    pad.taps(&mut e, &[Key::Right]);
    assert!(e.view(pad.t).buttons[1].focused);
    pad.taps(&mut e, &[Key::Left, Key::Up, Key::Left, Key::Down]);
    let v = e.view(pad.t);
    assert_eq!((v.selected, v.focus), ("toast", "buttons"));
    assert!(v.buttons[2].focused && v.buttons.iter().filter(|b| b.focused).count() == 1, "Done, as the toasts have no Full width");
}

#[test]
fn moving_the_status_bar_by_key_or_pointer_keeps_its_glass_spread() {
    let (mut e, mut pad) = (editor_on(json!({"statusbar": {"anchor": "top", "dx": 0, "dy": 0, "full": true}})), Pad::new());
    e.start(Some("statusbar"));
    let before = rect(&e, "statusbar", pad.t);
    pad.taps(&mut e, &[Key::Ok, Key::Left]);
    let out = saved(&pad.taps(&mut e, &[Key::Ok]));
    let p = &out["statusbar"];
    assert_eq!(p["full"], json!(true), "{out}");
    assert!(rect(&e, "statusbar", pad.t).x < before.x, "a press of Left moved the buttons");
    assert_eq!(glass(&e, "statusbar", pad.t), Rect::new(64, 54, 1792, 60), "and the glass stayed across the bar");
    // the pointer takes it anywhere on the glass, outside the buttons too, and moves the buttons by as far as it goes
    let at = rect(&e, "statusbar", pad.t);
    let (px, py) = (200.0, 84.0);
    assert!(f64::from(at.x) > px + 100.0, "the press is on the glass, left of the buttons");
    e.pointer_down("statusbar", px, py);
    e.pointer_move(px + 300.0, py, true, pad.t + 1);
    let out = saved(&e.pointer_up());
    assert_eq!(out["statusbar"]["full"], json!(true), "{out}");
    assert!(rect(&e, "statusbar", pad.t + 2).x > at.x + 200, "the buttons followed the pointer: {at:?} -> {:?}", rect(&e, "statusbar", pad.t + 2));
    assert_eq!(glass(&e, "statusbar", pad.t + 2), Rect::new(64, 54, 1792, 60));
}
