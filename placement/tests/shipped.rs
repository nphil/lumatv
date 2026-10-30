//! The shipped data and the model say the same: the margins the QML shell reads from `theme.json` and Home's `layout.json` are the model's, so a
//! pop-up and the cards end where Home's right edge does (they ended 32 px short of it while `overlays.right` was 96 and Home's safe area 64).

use lumatv_placement::model::STACK;
use lumatv_placement::{Frame, Registry, Size, User};
use serde_json::Value;

fn data(name: &str) -> Value {
    let path = format!("{}/../payload/theme/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap()
}

#[test]
fn the_models_fallbacks_are_the_shipped_layout() {
    let (layout, theme) = (data("layout.json"), data("theme.json"));
    let f = Frame::from_json(&layout, theme["overlays"].get("limit"));
    assert_eq!(f, Frame::default());
    assert_eq!(
        (f.safe_x, f.safe_y, f.bar, f.top, f.limit),
        (64, 54, 60, 134, 810)
    );
}

#[test]
fn the_pop_up_and_the_cards_sit_half_of_homes_safe_margin_from_the_corner() {
    let (layout, theme) = (data("layout.json"), data("theme.json"));
    let o = &theme["overlays"];
    // the QML shell's column: `overlays.top` and `overlays.right` from the screen's edges (half of the larger of Home's safe margins, 32 of 64), `overlays.width` wide
    let f = Frame::from_json(&layout, o.get("limit"));
    assert_eq!(
        o["top"].as_i64(),
        Some(i64::from(f.column_inset())),
        "overlays.top is the column's inset"
    );
    assert_eq!(
        o["right"].as_i64(),
        Some(i64::from(f.column_inset())),
        "overlays.right is the column's inset, 32"
    );
    assert_eq!(o["gap"].as_i64(), Some(i64::from(STACK)));
    let w = o["width"].as_i64().unwrap() as i32;
    let reg = Registry::shipped();
    for id in ["popup", "cards", "toast"] {
        assert_eq!(
            reg.item(id).unwrap().sample.w,
            w,
            "{id} is as wide as the cards"
        );
    }
    let r = reg
        .rect_of("popup", &User::default(), &f, Size::new(w, 469))
        .unwrap();
    assert_eq!(
        (r.right(), r.y),
        (
            1920 - o["right"].as_i64().unwrap() as i32,
            o["top"].as_i64().unwrap() as i32
        )
    );
}

#[test]
fn nothing_is_placed_in_the_shipped_theme() {
    let theme = data("theme.json");
    let reg = Registry::shipped();
    assert!(reg.user_from_json(&theme["layout"]["overlays"]).is_empty());
}
