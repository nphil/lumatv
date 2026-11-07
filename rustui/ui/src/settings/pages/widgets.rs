//! Home widgets (`settings/PageWidgets.qml`, a page of Home & look): which cards Home shows, how wide each is, and in what order. A row per
//! card, Off, Small, Half or Wide (four, six or eight of Home's twelve columns); a card that does not fit where it is says what is in the
//! way, and stays as it was. Order trades a card's place with the one before or after it in reading order. Every change is a whole layout
//! built with Home's grid (`home/grid.rs`) and handed to the shell, which has Home write the owner's `layout.json` (never the shipped
//! one) and apply it at once. "Edit on Home" opens the layout editor, where a card is moved and sized by the cell and new ones are added.

use crate::home::grid::{self, ADDABLE, Entry, Refusal, SIZES};
use crate::settings::ctx::Ctx;
use crate::settings::page::Page;
use crate::settings::row::{Arg, Op, Row, ShellRequest};
use serde_json::{Value, json};

/// How long a refusal is said in the row it was asked in.
const NOTICE_MS: u64 = 3500;

pub fn page() -> Page {
    Page::new("widgets", "Home widgets", "layout-grid").parent("look").back("Home & look").tabs(&["Show", "Order"]).rows(rows).summary(summary)
}

fn cards(layout: &Value) -> Vec<Entry> {
    grid::entries(layout).into_iter().filter(|e| !e.is_dock()).collect()
}

fn summary(c: &Ctx) -> String {
    format!("{} shown", cards(c.layout).iter().filter(|e| !e.hidden).count())
}

/// The types Home could add that it has no card of.
fn missing(layout: &Value) -> Vec<&'static str> {
    let have: Vec<String> = grid::entries(layout).into_iter().map(|e| e.kind).collect();
    ADDABLE.iter().copied().filter(|t| !have.iter().any(|h| h == t)).collect()
}

/// A card's name: its own title, else its type's.
fn title_of(layout: &Value, id: &str, kind: &str) -> String {
    match grid::find(layout, id) {
        Some(e) => e.title(),
        None => grid::type_of(kind).map_or_else(|| kind.to_string(), |t| t.title.to_string()),
    }
}

fn reason(r: &Refusal) -> String {
    match r {
        Refusal::Overlap(b) => format!("{} is in the way", b.title()),
        Refusal::Full => "No room".into(),
        Refusal::Small(_) => "Too small".into(),
        _ => "Not here".into(),
    }
}

/// What a refused change says, for a few seconds, in the row it was asked in.
fn say(id: &str, words: &str) -> Vec<Op> {
    vec![Op::Set("notice".into(), json!({ id: words })), Op::Later { key: "widgets:notice".into(), ms: NOTICE_MS, then: vec![Op::Set("notice".into(), json!({}))] }]
}

fn change(layout: Value) -> Op {
    Op::Shell(ShellRequest::HomeLayout(layout))
}

/// 0 Off, 1 Small, 2 Half, 3 Wide; a width that is none of the three (one set on Home) has no mark, and the row says the width instead.
fn size_index(e: Option<&Entry>) -> Option<usize> {
    let e = match e {
        Some(e) if !e.hidden => e,
        _ => return Some(0),
    };
    SIZES.iter().position(|s| s.0 == grid::size_of(e)).map(|i| i + 1)
}

/// What the row says under its name: a refusal while it lasts, else a width of its own ("7 of 12 columns").
fn detail(c: &Ctx, id: &str) -> String {
    let said = notice(c, id);
    match grid::find(c.layout, id) {
        Some(e) if said.is_empty() && size_index(Some(&e)).is_none() => format!("{} of {} columns", e.span, grid::grid(c.layout).0),
        _ => said,
    }
}

/// A size chosen for a card (`choose`): Off hides it; the others bring it back (a type with no card is added) and set its width.
fn choose(layout: &Value, id: &str, kind: &str, i: usize) -> Vec<Op> {
    // Home has not handed its layout over
    if layout.get("grid").is_none() {
        return Vec::new();
    }
    let existing = grid::find(layout, id);
    if i == 0 {
        return match existing {
            Some(e) if !e.hidden => match grid::hide(layout, id) {
                Ok(l) => vec![change(l)],
                Err(r) => say(id, &reason(&r)),
            },
            _ => Vec::new(),
        };
    }
    let (mut l, mut at) = (layout.clone(), id.to_string());
    match existing {
        None => match grid::add(&l, kind) {
            Ok((next, new_id)) => (l, at) = (next, new_id),
            Err(r) => return say(id, &reason(&r)),
        },
        Some(e) if e.hidden => match grid::show(&l, id) {
            Ok((next, _)) => l = next,
            Err(r) => return say(id, &reason(&r)),
        },
        Some(_) => {}
    }
    let mut ops = Vec::new();
    match grid::set_span(&l, &at, SIZES[i - 1].2) {
        Ok((next, _)) => l = next,
        // the card is on Home at the width it has; the one asked for does not fit
        Err(Refusal::Edge) => {}
        Err(r) => ops.extend(say(id, &reason(&r))),
    }
    if &l != layout {
        ops.push(change(l));
    }
    ops
}

fn nudge(layout: &Value, id: &str, d: i64) -> Vec<Op> {
    if layout.get("grid").is_none() {
        return Vec::new();
    }
    match grid::nudge(layout, id, d) {
        Ok(l) => vec![change(l)],
        Err(Refusal::Edge) => Vec::new(),
        Err(r) => say(id, &reason(&r)),
    }
}

/// Whether the owner has a layout of their own (what the shipped one is compared with is what Home would write).
fn customised(layout: &Value) -> bool {
    grid::for_file(layout, &grid::shipped()).as_object().is_some_and(|o| !o.is_empty())
}

fn notice(c: &Ctx, id: &str) -> String {
    c.local.get("notice").get(id).and_then(Value::as_str).unwrap_or("").to_string()
}

fn rows(c: &Ctx) -> Vec<Row> {
    let layout = c.layout;
    let mut items: Vec<(String, String)> = cards(layout).into_iter().map(|e| (e.id, e.kind)).collect();
    for t in missing(layout) {
        items.push((grid::type_of(t).map_or(t, |k| k.first_id).to_string(), t.to_string()));
    }
    let mut rows = Vec::new();
    for (id, kind) in &items {
        let icon = grid::type_of(kind).map_or("layout-grid", |t| t.icon);
        let (lay, rid, rkind) = (layout.clone(), id.clone(), kind.clone());
        let mut r = Row::seg(&format!("w:{id}"), &["Off", "Small", "Half", "Wide"]).tab(0).icon(icon).label(&title_of(layout, id, kind)).index(size_index(grid::find(layout, id).as_ref())).act(move |a| match a {
            Arg::Index(i) => choose(&lay, &rid, &rkind, i),
            _ => Vec::new(),
        });
        r.detail = detail(c, id);
        rows.push(r);
    }
    rows.push(Row::link("edit").tab(0).icon("layout-grid").label("Edit on Home").does(vec![Op::Shell(ShellRequest::EditHome)]));
    let shipped = grid::shipped();
    rows.push(Row::seg("reset", &["Keep", "Reset"]).tab(0).icon("rotate-ccw").label("Reset to default").index(Some(0)).enabled(customised(layout)).act(move |a| match a {
        Arg::Index(1) => vec![change(shipped.clone())],
        _ => Vec::new(),
    }));
    let order = grid::order(layout);
    for e in cards(layout).into_iter().filter(|e| !e.hidden) {
        let (lay, id) = (layout.clone(), e.id.clone());
        let icon = grid::type_of(&e.kind).map_or("layout-grid", |t| t.icon);
        let place = order.iter().position(|o| *o == e.id).map_or(0, |i| i + 1);
        let mut r = Row::nudge(&format!("n:{}", e.id)).tab(1).icon(icon).label(&e.title()).value(&format!("{place} of {}", order.len())).act(move |a| match a {
            Arg::Nudge(d) => nudge(&lay, &id, i64::from(d)),
            _ => Vec::new(),
        });
        r.detail = notice(c, &e.id);
        rows.push(r);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn world() -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.layout = grid::tidy_layout(&grid::shipped());
        w
    }

    fn row<'a>(r: &'a [Row], id: &str) -> &'a Row {
        r.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("no row {id}: {:?}", r.iter().map(|x| &x.id).collect::<Vec<_>>()))
    }

    fn layout_of(ops: &[Op]) -> Option<&Value> {
        ops.iter().find_map(|o| match o {
            Op::Shell(ShellRequest::HomeLayout(l)) => Some(l),
            _ => None,
        })
    }

    #[test]
    fn the_shipped_layout_is_a_row_per_card_in_the_widths_it_has_and_is_not_customised() {
        let w = world();
        let c = w.ctx();
        let r = rows(&c);
        let shipped_cards = cards(&w.layout);
        assert!(!shipped_cards.is_empty());
        for e in &shipped_cards {
            let row = row(&r, &format!("w:{}", e.id));
            assert_eq!((row.kind, row.tab, row.options.len()), (Kind::Seg, 0, 4));
            assert_eq!(row.index, size_index(Some(e)), "{}", e.id);
        }
        assert!(!row(&r, "reset").enabled, "the shipped layout has nothing to reset");
        assert_eq!(summary(&c), format!("{} shown", shipped_cards.iter().filter(|e| !e.hidden).count()));
        let order = grid::order(&w.layout);
        assert_eq!(r.iter().filter(|x| x.tab == 1).count(), order.len(), "one nudge row per card shown");
        assert_eq!(row(&r, &format!("n:{}", order[0])).value, format!("1 of {}", order.len()));
    }

    #[test]
    fn a_width_set_on_home_marks_no_size_and_says_the_width_and_a_size_pressed_sets_it() {
        // the owner's layout on the set (2026-11-06): Clock and Smart home 7 of the 12 columns, Weather 5, At home hidden, no News or Cameras
        let mut w = world();
        w.layout = grid::tidy_layout(&json!({"grid": {"rows": 16}, "widgets": [
            {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 7, "rows": 4},
            {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 7, "rows": 5, "hidden": true},
            {"id": "weather", "type": "weather", "col": 7, "row": 0, "span": 5, "rows": 9},
            {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7},
            {"id": "onhome", "type": "onhome", "col": 0, "row": 4, "span": 7, "rows": 5}]}));
        let r = rows(&w.ctx());
        let shown: Vec<(&str, Option<usize>, &str)> =
            r.iter().filter(|x| x.tab == 0 && x.kind == Kind::Seg && x.id != "reset").map(|x| (x.label.as_str(), x.index, x.detail.as_str())).collect();
        assert_eq!(shown, [
            ("Clock", None, "7 of 12 columns"),
            ("At home", Some(0), ""),
            ("Weather", None, "5 of 12 columns"),
            ("Smart home", None, "7 of 12 columns"),
            ("News", Some(0), ""),
            ("Cameras", Some(0), "")
        ]);
        // Small sets Weather to 4 columns
        let ops = (row(&r, "w:weather").act.as_ref().unwrap())(Arg::Index(1));
        assert_eq!(grid::find(layout_of(&ops).expect("a size writes the layout"), "weather").unwrap().span, 4);
        // a refusal is said over the width while it lasts
        w.local.set("notice", json!({"clock": "Weather is in the way"}));
        assert_eq!(row(&rows(&w.ctx()), "w:clock").detail, "Weather is in the way");
    }

    #[test]
    fn off_hides_a_card_a_size_shows_it_again_and_a_missing_type_is_added_where_there_is_room() {
        let w = world();
        let id = grid::order(&w.layout)[0].clone();
        let kind = grid::find(&w.layout, &id).unwrap().kind;
        let ops = choose(&w.layout, &id, &kind, 0);
        let hidden = layout_of(&ops).expect("hiding writes the layout");
        assert!(grid::find(hidden, &id).unwrap().hidden);
        assert!(choose(hidden, &id, &kind, 0).is_empty(), "off on a card that is off does nothing");
        // back on at a width: shown, then sized
        let ops = choose(hidden, &id, &kind, 2);
        let back = layout_of(&ops).expect("showing writes the layout");
        let e = grid::find(back, &id).unwrap();
        assert!(!e.hidden);
        assert!(e.span == 6 || ops.iter().any(|o| matches!(o, Op::Set(..))), "half is 6 columns, or the refusal is said: {e:?}");
        // a layout Home has not handed over is left alone
        assert!(choose(&json!({}), &id, &kind, 1).is_empty());
    }

    #[test]
    fn a_refusal_is_said_in_the_row_for_a_few_seconds_and_the_layout_is_left_as_it_was() {
        let ops = say("clock", "Weather is in the way");
        let [Op::Set(k, v), Op::Later { ms, then, .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((k.as_str(), v), ("notice", &json!({"clock": "Weather is in the way"})));
        assert_eq!(*ms, 3500);
        assert!(matches!(&then[..], [Op::Set(k, v)] if k == "notice" && v == &json!({})));
        let mut w = world();
        w.local.set("notice", json!({"clock": "Weather is in the way"}));
        assert_eq!(row(&rows(&w.ctx()), "w:clock").detail, "Weather is in the way");
        assert_eq!(reason(&Refusal::Full), "No room");
        assert_eq!(reason(&Refusal::Edge), "Not here");
    }

    #[test]
    fn order_trades_a_card_with_its_neighbour_and_the_ends_say_nothing() {
        let w = world();
        let order = grid::order(&w.layout);
        if order.len() < 2 {
            return;
        }
        let ops = nudge(&w.layout, &order[0], 1);
        let swapped = layout_of(&ops).or_else(|| ops.iter().find(|o| matches!(o, Op::Set(..))).map(|_| &w.layout)).unwrap();
        if layout_of(&ops).is_some() {
            assert_eq!(grid::order(swapped)[1], order[0], "the first card is the second now");
        }
        assert!(nudge(&w.layout, &order[0], -1).is_empty(), "the first has nothing before it");
        let r = rows(&w.ctx());
        let first = row(&r, &format!("n:{}", order[0]));
        assert!(matches!(&(first.act.as_ref().unwrap())(Arg::Nudge(-1))[..], []));
    }

    #[test]
    fn edit_on_home_and_reset_ask_the_shell_and_a_changed_layout_can_be_put_back() {
        let mut w = world();
        let r = rows(&w.ctx());
        assert!(matches!(&(row(&r, "edit").act.as_ref().unwrap())(Arg::Press)[..], [Op::Shell(ShellRequest::EditHome)]));
        let id = grid::order(&w.layout)[0].clone();
        let kind = grid::find(&w.layout, &id).unwrap().kind;
        let ops = choose(&w.layout, &id, &kind, 0);
        w.layout = layout_of(&ops).unwrap().clone();
        let r = rows(&w.ctx());
        let reset = row(&r, "reset");
        assert!(reset.enabled, "the owner has a layout of their own now");
        assert!(layout_of(&(reset.act.as_ref().unwrap())(Arg::Index(0))).is_none(), "Keep does nothing");
        let back = (reset.act.as_ref().unwrap())(Arg::Index(1));
        assert_eq!(layout_of(&back), Some(&grid::shipped()));
    }
}
