//! Tests of [`super::row`]: press, cancel and activation, the two kinds of row, the toggle row's proposal, tiles that show another row
//! without making a layer, and rows at large text.

use super::row::*;
use super::{Component, Damage, SemanticsSink};
use crate::geometry::Frame;
use crate::input::TouchPhase::{Cancel, Down, Move, Up};
use crate::lucent::material::PressTreatment;
use crate::testing::{Harness, harness, touch};
use compositor_protocol::{Content, Request};

const AT: Frame = Frame::new(40.0, 100.0, 560.0, 72.0);

fn row(h: &mut Harness, model: &RowModel) -> Row {
    let r = Row::new(&mut h.ui, AT, 10, model).unwrap();
    h.commit();
    r
}

fn glass_layers(h: &Harness) -> usize {
    h.layers()
        .iter()
        .filter(|(_, c, _)| matches!(c, Content::Glass(_)))
        .count()
}

#[test]
fn a_read_only_row_has_no_glass_of_its_own_and_an_actionable_row_has_the_raised_pair() {
    let mut h = harness();
    let plain = row(
        &mut h,
        &RowModel::new(RowId(1), "Software version").detail("v1-20261004b"),
    );
    assert!(plain.surface().is_none());
    assert_eq!(glass_layers(&h), 0, "text only: it inherits the sheet");
    let button = row(
        &mut h,
        &RowModel::new(RowId(2), "Wi-Fi")
            .trailing(Trailing::Chevron)
            .actionable(),
    );
    let (rest, pressed) = button
        .surface()
        .expect("an actionable row is a button")
        .shown();
    assert_eq!(rest.corner_radius, 18.0);
    assert!(
        rest.depth.is_some_and(|d| !d.inset)
            && rest.sheen.is_some()
            && rest.shadows == [None, None]
    );
    assert!(
        pressed.is_some_and(|p| p.depth.is_some_and(|d| !d.inset)),
        "a row presses by scale, keeping its pair"
    );
    assert!(matches!(
        button.surface().unwrap().pair().press,
        PressTreatment::Scale { .. }
    ));
    assert_eq!(glass_layers(&h), 2);
}

#[test]
fn a_row_activates_on_release_inside_and_never_on_a_drag_out_a_cancel_or_a_read_only_row() {
    let mut h = harness();
    let mut r = row(&mut h, &RowModel::new(RowId(2), "Wi-Fi").actionable());
    assert_eq!(r.input(&touch(1, Down, 100.0, 130.0, 0), &mut h.ui), None);
    assert!(r.is_pressed());
    assert_eq!(
        r.input(&touch(1, Up, 100.0, 130.0, 40), &mut h.ui),
        Some(RowAction::Activated)
    );
    assert!(!r.is_pressed());

    r.input(&touch(1, Down, 100.0, 130.0, 100), &mut h.ui);
    r.input(&touch(1, Move, 100.0, 500.0, 116), &mut h.ui);
    assert!(!r.is_pressed());
    assert_eq!(r.input(&touch(1, Up, 100.0, 500.0, 130), &mut h.ui), None);
    r.input(&touch(1, Down, 100.0, 130.0, 200), &mut h.ui);
    assert_eq!(r.input(&touch(1, Cancel, 0.0, 0.0, 210), &mut h.ui), None);
    r.input(&touch(1, Down, 100.0, 130.0, 300), &mut h.ui);
    r.cancel(&h.ui);
    assert!(!r.is_pressed());
    assert_eq!(r.input(&touch(1, Up, 100.0, 130.0, 310), &mut h.ui), None);

    let mut read_only = row(&mut h, &RowModel::new(RowId(3), "Serial"));
    assert_eq!(
        read_only.input(&touch(1, Down, 100.0, 130.0, 0), &mut h.ui),
        None
    );
    assert_eq!(
        read_only.input(&touch(1, Up, 100.0, 130.0, 20), &mut h.ui),
        None
    );
    let mut off = row(
        &mut h,
        &RowModel::new(RowId(4), "Forget network")
            .actionable()
            .enabled(false),
    );
    assert_eq!(off.input(&touch(1, Down, 100.0, 130.0, 0), &mut h.ui), None);
    assert!(!off.is_pressed());
}

#[test]
fn a_toggle_row_proposes_the_opposite_and_keeps_showing_the_confirmed_value() {
    let mut h = harness();
    let model = RowModel::new(RowId(5), "Show seconds")
        .trailing(Trailing::Toggle(false))
        .actionable();
    let mut r = row(&mut h, &model);
    r.input(&touch(1, Down, 100.0, 130.0, 0), &mut h.ui);
    assert_eq!(
        r.input(&touch(1, Up, 100.0, 130.0, 30), &mut h.ui),
        Some(RowAction::Proposed(true))
    );
    assert_eq!(r.model().trailing, Trailing::Toggle(false));
    let mut sink = SemanticsSink::new();
    r.semantics(&mut sink);
    let node = sink.find("Show seconds").unwrap();
    assert_eq!(node.value.as_deref(), Some("Off"));
    assert!(!node.selected);
    // Confirmed: the knob moves and the row's text layer draws the tick (a mark of it: the words' layer is drawn again, as a changed word
    // draws it), without a new layer.
    let layers = h.layers().len();
    let on = RowModel::new(RowId(5), "Show seconds")
        .trailing(Trailing::Toggle(true))
        .actionable();
    assert_eq!(r.update(&on, &mut h.ui), Damage::Content);
    h.commit();
    assert_eq!(h.layers().len(), layers);
    let mut sink = SemanticsSink::new();
    r.semantics(&mut sink);
    assert!(sink.find("Show seconds").unwrap().selected);
}

#[test]
fn a_tile_shows_another_row_by_drawing_text_and_makes_no_layer() {
    let mut h = harness();
    let needs = Needs {
        glass: true,
        toggle: true,
        trail_w: 120.0,
        ..Needs::default()
    };
    let mut tile = Row::spare(&mut h.ui, AT, 10, needs).unwrap();
    h.commit();
    assert!(!tile.is_shown());
    let layers = h.layers().len();
    let a = RowModel::new(RowId(7), "Guest network")
        .detail("Secured")
        .trailing(Trailing::Value("Strong".into()))
        .actionable();
    tile.try_update(&a, &mut h.ui).unwrap();
    tile.place(&h.ui, 300.0);
    assert!(tile.is_shown() && tile.id() == RowId(7));
    let b = RowModel::new(RowId(8), "Kitchen")
        .trailing(Trailing::Toggle(true))
        .actionable();
    tile.try_update(&b, &mut h.ui).unwrap();
    tile.place(&h.ui, 380.0);
    tile.hide(&h.ui);
    assert!(!tile.is_shown());
    h.commit();
    assert_eq!(
        h.layers().len(),
        layers,
        "a tile that was given the layers every row needs never makes another"
    );
}

#[test]
fn a_row_given_a_model_that_needs_more_than_it_has_grows_the_layers_it_lacks() {
    let mut h = harness();
    let mut r = row(&mut h, &RowModel::new(RowId(1), "Plain"));
    assert_eq!(glass_layers(&h), 0);
    let button = RowModel::new(RowId(1), "Plain")
        .trailing(Trailing::Toggle(true))
        .actionable();
    assert!(!r.needs().covers(&Row::needs_of(&button)));
    r.try_update(&button, &mut h.ui).unwrap();
    h.commit();
    assert_eq!(
        glass_layers(&h),
        4,
        "the pair of the row and a track and a knob"
    );
    assert!(r.needs().glass && r.needs().toggle);
}

/// Nitin's decision 36 (10-08): a row's toggle is the approved mock-ups' `.sw`: a track of 68 by 40 at the row's right end, a light
/// knob of 30 five in from the end of the track (the layers a toggle always had: the glass of the track and of the knob, under the
/// row's text layer), and, while it is on, the accent tick over the knob: a mark of that text layer, so no layer and no picture is made
/// for it (the settings' layer ceiling and the entrance's pictures are pinned).
#[test]
fn a_rows_toggle_is_the_mocks_sw_a_track_of_68_by_40_a_light_knob_of_30_and_a_tick_in_the_text_layer_while_on()
 {
    let mut h = harness();
    let off = RowModel::new(RowId(5), "Show seconds")
        .trailing(Trailing::Toggle(false))
        .actionable();
    let mut r = row(&mut h, &off);
    let created = |h: &Harness| -> Vec<(u32, u32, Content, compositor_protocol::Props, i32, _)> {
        h.all()
            .into_iter()
            .filter_map(|r| match r {
                Request::Create {
                    id,
                    width,
                    height,
                    content,
                    props,
                    z,
                } => Some((width, height, content, props, z, id)),
                _ => None,
            })
            .collect()
    };
    let layers = created(&h);
    // The track: 68 by 40, 20 from the row's right end and in the middle of its height (AT is 40, 100, 560 by 72), at the row's depth.
    let track = layers
        .iter()
        .find(|l| (l.0, l.1) == (68, 40))
        .expect("a track of 68 by 40");
    assert!(matches!(track.2, Content::Glass(_)));
    assert_eq!(
        (track.3.x, track.3.y, track.4),
        (AT.right() - 20.0 - 68.0, 116.0, 10)
    );
    // The knob: 30, 5 in from the track's end and from its top, in the light colour, at the same depth and made after the track.
    let knob = layers
        .iter()
        .find(|l| (l.0, l.1) == (30, 30) && matches!(l.2, Content::Glass(_)))
        .expect("a knob of 30");
    assert_eq!(
        (knob.3.x, knob.3.y, knob.4),
        (track.3.x + 5.0, 116.0 + 5.0, 10)
    );
    let light = h.ui.look.theme.materials.knob;
    assert!(
        matches!(knob.2, Content::Glass(g) if g.tint == light),
        "a light knob"
    );
    // The words' layer (520 by 72) is above both: the tick is its mark.
    let words = layers
        .iter()
        .find(|l| (l.0, l.1) == (520, 72) && matches!(l.2, Content::Image))
        .expect("the row's text layer, a picture on this host");
    assert!(
        words.4 > knob.4,
        "the tick, a mark of this layer, is over the knob"
    );
    // The tick's place in that layer: the knob's middle at the far end, 18 square; clear while the toggle is off.
    let tick_ink = |pixels: &[u8]| {
        let (x0, y0) = (520 - 5 - 15 - 9, (72 - 18) / 2);
        (y0..y0 + 18)
            .flat_map(|y| (x0..x0 + 18).map(move |x| (x, y)))
            .filter(|&(x, y)| pixels[(y * 520 + x) * 4 + 3] >= 128)
            .count()
    };
    assert_eq!(tick_ink(&h.pixels(words.5)), 0, "no tick while off");
    // On: the knob goes to the other end, the words' layer draws the tick, and nothing is made.
    let on = RowModel::new(RowId(5), "Show seconds")
        .trailing(Trailing::Toggle(true))
        .actionable();
    assert_eq!(r.update(&on, &mut h.ui), Damage::Content);
    h.commit();
    let now = h.layers();
    let moved = now.iter().find(|l| l.0 == knob.5).expect("the same layer");
    assert_eq!(moved.2.x, track.3.x + 5.0 + 28.0, "28 along its track");
    assert!(
        tick_ink(&h.pixels(words.5)) > 20,
        "the tick is drawn while on"
    );
    assert_eq!(created(&h).len(), layers.len(), "turning it made no layer");
}

#[test]
fn rows_are_at_least_64_high_and_large_text_makes_them_taller_and_never_the_words_shorter() {
    let mut h = harness();
    let model = RowModel::new(
        RowId(1),
        "A network whose name is long enough to need a second line to be read",
    )
    .detail("Secured with WPA2, connected on 5 GHz, strong signal")
    .trailing(Trailing::Chevron)
    .actionable();
    let short = Row::required_height(&mut h.ui, &RowModel::new(RowId(2), "Ok"), AT.w);
    assert_eq!(short, 64.0);
    let normal = Row::required_height(&mut h.ui, &model, AT.w);
    h.ui.look.text_scale = 2.0;
    let large = Row::required_height(&mut h.ui, &model, AT.w);
    assert!(large > normal * 1.4, "{normal} -> {large}");
    let frame = Frame::new(0.0, 0.0, AT.w, large);
    let r = Row::new(&mut h.ui, frame, 10, &model).unwrap();
    let (title, detail) = r.shown_lines();
    assert!(title.len() <= MAX_TITLE_LINES && detail.len() <= MAX_DETAIL_LINES);
    assert!(title.iter().chain(detail).all(|l| !l.is_empty()));
    assert!(r.hit().h >= 64.0);
    h.commit();
}

#[test]
fn a_changed_title_draws_the_text_layer_once_and_a_state_change_draws_nothing() {
    let mut h = harness();
    let mut r = row(&mut h, &RowModel::new(RowId(1), "Wi-Fi").actionable());
    let renamed = RowModel::new(RowId(1), "Wireless").actionable();
    assert_eq!(r.update(&renamed, &mut h.ui), Damage::Content);
    assert_eq!(r.update(&renamed, &mut h.ui), Damage::None);
    let sent = h.commit();
    assert_eq!(
        sent.iter()
            .filter(|q| matches!(q, Request::Damage { .. }))
            .count(),
        1
    );
    let selected = renamed.clone().selected(true);
    assert_eq!(r.update(&selected, &mut h.ui), Damage::Properties);
    let sent = h.commit();
    assert!(
        !sent
            .iter()
            .any(|q| matches!(q, Request::Damage { .. } | Request::Create { .. })),
        "{sent:?}"
    );
}

#[test]
fn the_part_of_a_row_outside_the_window_is_clipped_and_semantics_name_the_visible_part() {
    let mut h = harness();
    let mut r = row(&mut h, &RowModel::new(RowId(1), "Clipped").actionable());
    let window = compositor_protocol::Rect {
        x: 40.0,
        y: 130.0,
        w: 560.0,
        h: 200.0,
    };
    r.set_clip(Some(window));
    let mut sink = SemanticsSink::new();
    r.semantics(&mut sink);
    let node = sink.find("Clipped").unwrap();
    assert!(node.frame.y >= 130.0 - 0.01, "{:?}", node.frame);
}

/// Every look, state and trailing mark a row has, in one list.
fn every_row() -> Vec<RowModel> {
    use super::icon::Icon;
    let trailing = [
        Trailing::None,
        Trailing::Chevron,
        Trailing::Check(true),
        Trailing::Value("80%".into()),
        Trailing::Signal {
            bars: 3,
            locked: true,
        },
        Trailing::Icon(Icon::Lock),
    ];
    let mut out = Vec::new();
    for look in [
        RowLook::Flat,
        RowLook::Card,
        RowLook::Tile,
        RowLook::Rail,
        RowLook::Plain,
    ] {
        for leading in [None, Some(Icon::Sun)] {
            for enabled in [true, false] {
                for selected in [false, true] {
                    for t in &trailing {
                        // A network's signal stands beside words only: with an icon before them, and the row's own ink, accent and
                        // hairline, it would be five colours, which a layer of text cannot be (a picture is made of it).
                        if matches!(t, Trailing::Signal { .. }) && leading.is_some() {
                            continue;
                        }
                        let mut m = RowModel::new(RowId(out.len() as u64), "Display")
                            .detail("Brightness and sleep")
                            .look(look)
                            .trailing(t.clone())
                            .enabled(enabled)
                            .selected(selected);
                        if let Some(icon) = leading {
                            m = m.leading(icon);
                        }
                        out.push(m);
                    }
                }
            }
        }
    }
    // A settings page's row with a tile (the Presence page's `.row` with its `.ich`): the square and its glyph in the state's ink, the title,
    // the detail, a value, a count, a toggle or nothing at the end, and the hairline. The square is a veil of the glyph's ink, which makes
    // four colours; as a colour of its own it was five, and these rows were pictures a moment after they were runs.
    for tint in [
        Some(ValueTone::Good),
        Some(ValueTone::Warn),
        Some(ValueTone::Bad),
        None,
    ] {
        for look in [RowLook::Flat, RowLook::Plain] {
            for t in [
                Trailing::None,
                Trailing::Value("Off".into()),
                Trailing::Count("3".into()),
                Trailing::Toggle(true),
                Trailing::Toggle(false),
            ] {
                // The tick over a toggle that is on is the accent: beside a square in another colour (a warning's, a danger's, the quiet
                // ink: a Presence row holds its state with the accent and is quiet when it is off) it is a fifth ink, which a layer of
                // text cannot have (a picture is made of it).
                if matches!(t, Trailing::Toggle(true)) && tint != Some(ValueTone::Good) {
                    continue;
                }
                out.push(
                    RowModel::new(RowId(out.len() as u64), "Camera")
                        .detail("Not in use")
                        .look(look)
                        .trailing(t)
                        .dress(Dress {
                            tile: super::Icon::Person,
                            tint,
                            tag: None,
                            status: None,
                            min_height: 72.0,
                            field: true,
                        }),
                );
            }
        }
    }
    out
}

fn text_layer(row: &Row) -> &super::TextLayer {
    row.group()
        .parts
        .iter()
        .find_map(|p| match &p.kind {
            super::Kind::Lines(layer) => Some(layer),
            _ => None,
        })
        .expect("a row's words are a text layer")
}

/// Answers every upload of the atlas, as the compositor does, until nothing is left to hand over.
fn settle(h: &mut Harness) {
    for _ in 0..80 {
        let mut any = false;
        for request in h.commit() {
            if let Request::AtlasUpload {
                id,
                slot,
                generation,
                ..
            } = request
            {
                h.ui.scene
                    .observe(&compositor_protocol::Event::AtlasUploaded {
                        id,
                        slot,
                        generation,
                    });
                any = true;
            }
        }
        if !any {
            return;
        }
    }
}

#[test]
fn a_row_in_every_look_and_state_is_one_text_layer_that_is_the_same_pixels_as_a_run_and_as_a_picture()
 {
    let models = every_row();
    let mut picture = harness();
    let mut glyph = harness();
    glyph.ui.enable_glyph_text().unwrap();
    // The atlas has seen the page before, as one that showed it once has. (A batch of rows at a time: a scene holds at most 256
    // layers.)
    for batch in models.chunks(30) {
        let rows: Vec<Row> = batch
            .iter()
            .map(|m| Row::new(&mut glyph.ui, AT, 10, m).unwrap())
            .collect();
        settle(&mut glyph);
        drop(rows);
        glyph.commit();
    }
    for batch in models.chunks(30) {
        let pictures: Vec<Row> = batch
            .iter()
            .map(|m| Row::new(&mut picture.ui, AT, 10, m).unwrap())
            .collect();
        picture.commit();
        let runs: Vec<Row> = batch
            .iter()
            .map(|m| Row::new(&mut glyph.ui, AT, 10, m).unwrap())
            .collect();
        let sent = glyph.commit();
        let made = |f: fn(&Request) -> bool| sent.iter().filter(|r| f(r)).count();
        assert_eq!(
            made(|r| matches!(
                r,
                Request::Create {
                    content: Content::Image,
                    ..
                }
            )),
            0,
            "no picture"
        );
        assert_eq!(
            made(|r| matches!(r, Request::Damage { .. })),
            0,
            "no pixel written"
        );
        assert_eq!(glyph.ui.glyph_text_fallbacks, 0);
        for ((model, p), g) in batch.iter().zip(&pictures).zip(&runs) {
            let (pl, gl) = (text_layer(p), text_layer(g));
            assert!(!pl.is_glyph_run());
            assert!(gl.is_glyph_run(), "{model:?} needs more than four colours");
            let layers = |row: &Row| row.group().parts.len();
            assert_eq!(layers(p), layers(g), "{model:?}");
            assert_eq!(
                picture.layer_pixels(pl.id()),
                glyph.layer_pixels(gl.id()),
                "{model:?}"
            );
        }
        drop((pictures, runs));
        picture.commit();
        glyph.commit();
    }
}

#[test]
fn a_flat_row_with_an_icon_a_value_and_a_hairline_is_one_layer_and_a_change_of_its_value_is_one_swap()
 {
    use super::icon::Icon;
    let model = RowModel::new(RowId(1), "Display")
        .detail("Brightness and sleep")
        .look(RowLook::Flat)
        .leading(Icon::Sun)
        .trailing(Trailing::Value("80%".into()));
    let mut h = harness();
    h.ui.enable_glyph_text().unwrap();
    let warm = Row::new(&mut h.ui, AT, 10, &model).unwrap();
    settle(&mut h);
    drop(warm);
    h.commit();
    let mut row = Row::new(&mut h.ui, AT, 10, &model).unwrap();
    let sent = h.commit();
    assert_eq!(
        row.group().parts.len(),
        1,
        "the words, the icon, the value and the hairline"
    );
    assert_eq!(
        sent.iter()
            .filter(|r| matches!(r, Request::Create { .. }))
            .count(),
        1
    );
    let id = text_layer(&row).id();
    let mut changed = model.clone();
    changed.trailing = Trailing::Value("65%".into());
    assert_eq!(row.update(&changed, &mut h.ui), Damage::Content);
    let swap = h.commit();
    assert_eq!(
        swap.iter()
            .filter(|r| matches!(r, Request::SetGlyphs { id: set, .. } if *set == id))
            .count(),
        1,
        "{swap:?}"
    );
    assert!(
        swap.iter()
            .all(|r| !matches!(r, Request::Create { .. } | Request::Damage { .. })),
        "{swap:?}"
    );
}

#[test]
fn a_chip_and_a_count_at_a_rows_end_are_in_its_one_text_layer_and_say_their_words() {
    let chip = |text: &str, tone, check| Trailing::Chip {
        text: text.into(),
        tone,
        check,
    };
    let model = |t: Trailing| {
        RowModel::new(RowId(1), "10:05 PM today")
            .detail("Marked here \u{b7} 1.2 MB")
            .look(RowLook::Flat)
            .trailing(t)
    };
    let mut h = harness();
    h.ui.enable_glyph_text().unwrap();
    // The atlas has the words of both pills before the row is made, as it has by the time a page is shown.
    for words in ["On the NAS", "On the Show only"] {
        let warm = Row::new(&mut h.ui, AT, 10, &model(chip(words, None, true))).unwrap();
        settle(&mut h);
        drop(warm);
        h.commit();
    }
    // The pill, its tick and its words are the row's one layer, as the value of a plain row is.
    let mut row = Row::new(
        &mut h.ui,
        AT,
        10,
        &model(chip("On the NAS", Some(ValueTone::Good), true)),
    )
    .unwrap();
    let sent = h.commit();
    assert_eq!(row.group().parts.len(), 1);
    assert_eq!(
        sent.iter()
            .filter(|r| matches!(r, Request::Create { .. }))
            .count(),
        1
    );
    // A change of the words is the one layer drawn again (the pill is a shape, which the glyph path has not): no layer is made.
    let id = text_layer(&row).id();
    assert_eq!(
        row.update(&model(chip("On the Show only", None, false)), &mut h.ui),
        Damage::Content
    );
    let swap = h.commit();
    assert_eq!(
        swap.iter()
            .filter(|r| matches!(r, Request::Damage { id: d, .. } if *d == id))
            .count(),
        1,
        "{swap:?}"
    );
    assert!(
        swap.iter().all(|r| !matches!(r, Request::Create { .. })),
        "{swap:?}"
    );
    // What a chip or a count says is what the row is described by, with its detail.
    for (trailing, said) in [
        (
            chip("On the Show only", None, false),
            "On the Show only, Marked here \u{b7} 1.2 MB",
        ),
        (Trailing::Count("3".into()), "3, Marked here \u{b7} 1.2 MB"),
    ] {
        let row = Row::new(&mut h.ui, AT, 10, &model(trailing)).unwrap();
        let mut sink = SemanticsSink::new();
        row.semantics(&mut sink);
        let nodes = sink.into_nodes();
        assert_eq!(nodes[0].value.as_deref(), Some(said), "{nodes:?}");
    }
}

#[test]
fn a_long_chip_is_cut_to_half_the_row_and_never_wraps_the_words_under_it() {
    let mut h = harness();
    let height = |h: &mut Harness, t: Trailing| {
        Row::required_height(
            &mut h.ui,
            &RowModel::new(RowId(1), "Oct 5 \u{b7} 8:12 PM")
                .detail("Volume keys")
                .look(RowLook::Flat)
                .trailing(t),
            AT.w,
        )
    };
    let chip = |text: &str| Trailing::Chip {
        text: text.into(),
        tone: None,
        check: true,
    };
    // The words of the row keep their lines whatever the chip says: it is cut with an ellipsis to its room.
    let long = "x".repeat(200);
    assert_eq!(
        height(&mut h, chip(&long)),
        height(&mut h, chip("On the NAS"))
    );
    assert!(height(&mut h, Trailing::Count("3".into())) >= 64.0);
}

/// A veil of an opaque tint is the square a colour of its own made (the tint at `wash` of 255, premultiplied), to within a unit of
/// rounding at the pixels of its edge: the look of the tile is what it was, and the square is no ink more in the row's run.
#[test]
fn the_tinted_square_is_a_veil_of_an_opaque_tint_the_pixels_a_colour_of_its_own_gave_to_within_a_unit()
 {
    use super::mark::Mark;
    use compositor_protocol::Rgba;
    let tint = Rgba(169, 157, 245, 255);
    for (side, radius, wash) in [
        (48u32, 14u32, 41u8),
        (48, 12, 48),
        (56, 16, 40),
        (48, 14, 51),
    ] {
        let scale = |c: u8| (u32::from(c) * u32::from(wash) / 255) as u8;
        let own = Rgba(scale(tint.0), scale(tint.1), scale(tint.2), wash);
        let n = side as usize;
        let paint = |mark: Mark| {
            let mut px = vec![0u8; n * n * 4];
            mark.paint(&mut px, n);
            px
        };
        let old = paint(Mark::round(0, 0, side, side, radius, own));
        let new = paint(Mark::veil(0, 0, side, side, radius, wash, tint));
        let most = old
            .iter()
            .zip(&new)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(most <= 1, "{side} {radius} {wash}: {most} levels apart");
        assert_eq!(
            old[(n / 2 * n + n / 2) * 4..][..4],
            new[(n / 2 * n + n / 2) * 4..][..4],
            "the middle of the square is the same pixel"
        );
    }
}

fn dressed(status: Option<(&str, ValueTone, bool)>) -> RowModel {
    RowModel::new(RowId(40), "Countdowns")
        .look(RowLook::Flat)
        .actionable()
        .dress(Dress {
            tile: super::Icon::Puzzle,
            tint: status.map(|s| s.1),
            tag: Some("v0.2.0".to_owned()),
            status: status.map(|(text, tone, ring)| DressStatus {
                text: text.to_owned(),
                tone,
                ring,
            }),
            min_height: 84.0,
            field: false,
        })
}

#[test]
fn a_dressed_row_is_one_layer_with_its_square_its_tag_and_its_status_line_and_is_at_least_as_high_as_it_says()
 {
    let mut h = harness();
    let model = dressed(Some((
        "Switched off \u{b7} paused twice in one day",
        ValueTone::Bad,
        false,
    )));
    let height = Row::required_height(&mut h.ui, &model, AT.w);
    assert_eq!(
        height, 84.0,
        "a list of things is taller than a list of settings"
    );
    let before = glass_layers(&h);
    let r = row(&mut h, &model);
    assert_eq!(
        glass_layers(&h) - before,
        0,
        "a flat row has no glass of its own, and the dress adds none"
    );
    let text_layers = h
        .layers()
        .iter()
        .filter(|(_, c, _)| matches!(c, Content::Image))
        .count();
    assert!(
        text_layers <= 1,
        "the square, the tag, the dot and the words are the row's one text layer: {text_layers}"
    );
    // A change of what the status says is one swap of that layer, and a ring for a dot is a change of the same layer.
    let mut r = r;
    r.update(
        &dressed(Some(("Off \u{b7} costs nothing", ValueTone::Muted, true))),
        &mut h.ui,
    );
    h.commit();
    // The same words in the same state change nothing.
    let again = r.update(
        &dressed(Some(("Off \u{b7} costs nothing", ValueTone::Muted, true))),
        &mut h.ui,
    );
    assert_eq!(again, Damage::None);
}

#[test]
fn a_row_may_say_how_high_it_least_is_and_a_reading_is_lower_than_a_touch_target() {
    let mut h = harness();
    let plain = RowModel::new(RowId(41), "Processor").look(RowLook::Plain);
    let target = Row::required_height(&mut h.ui, &plain, AT.w);
    let reading = Row::required_height(&mut h.ui, &plain.clone().min_height(36.0), AT.w);
    assert!(reading < target, "{reading} against {target}");
    assert!(reading >= 36.0);
}

/// The words' ink: the sum of the alpha of the row's text layer.
fn ink_of(h: &mut Harness, model: &RowModel) -> u64 {
    let r = row(h, model);
    h.layer_pixels(text_layer(&r).id())
        .chunks(4)
        .map(|px| u64::from(px[3]))
        .sum()
}

#[test]
fn a_page_row_is_72_high_with_a_detail_and_64_alone_as_the_mocks_row_is() {
    let mut h = harness();
    // The settings view draws at a fifth more than the person's size: a label of 24 and a detail of 20.
    h.ui.look.text_scale = 1.2;
    let high = |h: &mut Harness, m: &RowModel| Row::required_height(&mut h.ui, m, AT.w);
    for look in [RowLook::Flat, RowLook::Plain] {
        let alone = RowModel::new(RowId(60), "Running").look(look);
        assert_eq!(
            high(&mut h, &alone),
            64.0,
            "{look:?}: a label alone is a touch target"
        );
        let with = alone.clone().detail("Slot B · good · since 6:12 PM today");
        // 8 above and below, a line of 29 for the 24 px label (1.2), 2 between, a line of 25 for the 20 px detail (1.25).
        assert_eq!(high(&mut h, &with), 72.0, "{look:?}: the mock's row");
        let two = alone.detail(
            "A start that went wrong and was put right by the Show itself, as the line tells",
        );
        assert_eq!(
            high(&mut h, &two),
            97.0,
            "{look:?}: a second line of detail is another 25"
        );
    }
    // The toolkit's other looks keep Lucent's running text: a 1.4 line, with nothing between.
    let card = RowModel::new(RowId(61), "Running").detail("Slot B · good · since 6:12 PM today");
    assert_eq!(high(&mut h, &card), 78.0);
}

#[test]
fn a_page_rows_label_is_set_heavier_than_running_text() {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    let words = |look| RowModel::new(RowId(62), "Other system").look(look);
    let page = ink_of(&mut h, &words(RowLook::Plain));
    let running = ink_of(&mut h, &words(RowLook::Card));
    // The mock's label is at weight 550: SemiBold (600) for the Show, which has 400, 500 and 600. About a third more ink than Regular.
    assert!(page * 100 >= running * 120, "{page} against {running}");
    assert!(page * 100 <= running * 160, "{page} against {running}");
}

#[test]
fn a_row_of_a_things_layer_is_the_mocks_perm_21_over_18_on_1_2_lines_and_a_reading_is_its_meter() {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    h.ui.look.page = true;
    let high = |h: &mut Harness, m: &RowModel| Row::required_height(&mut h.ui, m, 458.0);
    let tile = || Dress {
        tile: crate::components::Icon::Check,
        tint: None,
        tag: None,
        status: None,
        min_height: 60.0,
        field: false,
    };
    let perm = |title: &str, detail: &str| {
        RowModel::new(RowId(70), title)
            .look(RowLook::Compact)
            .detail_lines(3)
            .detail(detail)
            .dress(tile())
    };
    // 6 above and below, a 21 px title on a line of 25.2 and an 18 px detail on one of 21.6: 58.8, and the row is at least 60.
    let one = perm(
        "Items on your day line",
        "Things it wants you to see at their time",
    );
    assert_eq!(high(&mut h, &one), 60.0);
    // Two lines of detail: 12 + 25.2 + 2 x 21.6 = 80.4.
    let two = perm(
        "It cannot",
        "Use the screen, camera, Bluetooth or network by itself",
    );
    assert_eq!(high(&mut h, &two), 81.0);
    // A meter: the label's line and 6 above and below it: 37.2, at least the 36 it is given.
    let meter = RowModel::new(RowId(71), "Processor")
        .look(RowLook::Compact)
        .trailing(Trailing::Value("0.1 % of 1 %".into()))
        .min_height(36.0);
    assert_eq!(high(&mut h, &meter), 38.0);
    // The room at its sides is the mock's 14 (a page's rows have 20).
    assert_eq!(pad_x(RowLook::Compact), 14.0);
    assert_eq!(pad_x(RowLook::Flat), PAD_X);
}

#[test]
fn a_spare_row_given_a_look_with_other_room_at_its_sides_has_its_words_where_that_look_puts_them() {
    // The text layer of a spare row is made before the row has a look (the toolkit's 20 at each side): a row shown in the look of a
    // thing's layer (14) or of a plate (none) has the layer moved and given the width the words were measured for, else they stand
    // 6 or 20 px from where the mock has them and are cut short at the end.
    for (look, pad) in [
        (RowLook::Compact, 14.0),
        (RowLook::State, 0.0),
        (RowLook::Flat, PAD_X),
    ] {
        let mut h = harness();
        h.ui.look.text_scale = 1.2;
        h.ui.look.page = true;
        let at = Frame::new(100.0, 40.0, 400.0, 60.0);
        let mut r = Row::spare(&mut h.ui, at, 10, Needs::default()).unwrap();
        r.try_update(
            &RowModel::new(RowId(1), "Processor")
                .look(look)
                .trailing(Trailing::Value("0.1 % of 1 %".into())),
            &mut h.ui,
        )
        .unwrap();
        h.commit();
        let words = r.group().parts[0].frame;
        assert_eq!(
            (words.x, words.w),
            (100.0 + pad, 400.0 - 2.0 * pad),
            "{look:?}"
        );
        let made: Vec<f32> = h
            .layers()
            .iter()
            .filter(|(_, c, _)| matches!(c, Content::Image | Content::Glyphs(_)))
            .map(|(_, _, p)| p.x)
            .collect();
        assert_eq!(made, [100.0 + pad], "{look:?}: one layer of words, there");
    }
}

#[test]
fn the_words_of_a_plate_are_23_over_20_on_lines_of_1_2_and_1_25_two_apart_with_no_room_of_their_own()
 {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    h.ui.look.page = true;
    let high = |h: &mut Harness, m: &RowModel| Row::required_height(&mut h.ui, m, 334.0);
    let words = |detail: Option<&str>| {
        let m = RowModel::new(RowId(5), "Running well")
            .look(RowLook::State)
            .detail_lines(3);
        match detail {
            Some(d) => m.detail(d),
            None => m,
        }
    };
    // The headline alone: one line of 27.6.
    assert_eq!(high(&mut h, &words(None)), 28.0);
    // And a line under it: 27.6 + 2 + 25.
    assert_eq!(high(&mut h, &words(Some("Using very little."))), 55.0);
    // The mock's sentence wraps to two lines in 334: 27.6 + 2 + 2 x 25 = 79.6, the plate's 103.6 with its 12 above and below.
    assert_eq!(
        high(
            &mut h,
            &words(Some("Using about a quarter of what it may."))
        ),
        80.0
    );
    assert_eq!(pad_x(RowLook::State), 0.0);
}

#[test]
fn a_pill_in_a_row_is_the_mocks_18_px_in_the_heavier_weight_and_grows_with_the_text_size() {
    // `.chipw`: 18 px at 600 on a pill of 32 with 12 at each side and an 18 px tick; the pill's other sizes were made for it.
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    h.ui.look.page = true;
    let t = chip_type(&h.ui);
    assert_eq!((t.px, t.weight), (18, crate::text::Weight::SemiBold));
    h.ui.look.text_scale = 1.8;
    assert_eq!(chip_type(&h.ui).px, 27);
}

#[test]
fn a_page_row_with_a_pill_at_its_end_sets_its_title_23_px_as_the_mocks_bug_rows_do() {
    // `.bug .tx .lb{font-size:23px}`: 8 + a line of 27.6 + 2 + 25 + 8 = the mock's 70.6 (71 here), where the other rows' 24 px
    // titles are 72.
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    h.ui.look.page = true;
    let plain = RowModel::new(RowId(9), "10:05 PM today")
        .look(RowLook::Flat)
        .detail("Marked here \u{b7} 1.2 MB");
    let pill = plain.clone().trailing(Trailing::Chip {
        text: "On the NAS".into(),
        tone: Some(ValueTone::Good),
        check: true,
    });
    assert_eq!(Row::required_height(&mut h.ui, &plain, 904.0), 72.0);
    assert_eq!(Row::required_height(&mut h.ui, &pill, 904.0), 71.0);
}

#[test]
fn a_reading_against_its_limit_is_said_whole_drawn_in_one_layer_and_cut_whole_when_it_must_be() {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    h.ui.look.page = true;
    let at = Frame::new(506.0, 100.0, 424.0, 54.0);
    let model = RowModel::new(RowId(3), "Processor")
        .look(RowLook::Compact)
        .trailing(Trailing::Limit {
            value: "0.1 %".into(),
            of: "of 1 %".into(),
        });
    let row = Row::new(&mut h.ui, at, 10, &model).unwrap();
    h.commit();
    // What a screen reader says is the words as they read, the two inks being only the look.
    let mut sink = SemanticsSink::new();
    row.semantics(&mut sink);
    assert_eq!(sink.into_nodes()[0].value.as_deref(), Some("0.1 % of 1 %"));
    // The reading and its limit are the row's one text layer (the row's own and the limit's runs).
    let layers = h
        .layers()
        .iter()
        .filter(|(_, c, _)| matches!(c, Content::Image | Content::Glyphs(_)))
        .count();
    assert_eq!(layers, 1);
}

/// A tile as a page names one (`Field.icon`): the glyph on the accent's square.
fn tile(tint: Option<ValueTone>) -> Dress {
    Dress {
        tile: super::Icon::Person,
        tint,
        tag: None,
        status: None,
        min_height: 0.0,
        field: true,
    }
}

#[test]
fn the_tile_of_a_row_is_the_mocks_ich_48_square_14_round_and_16_percent_of_its_tone() {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    let model = |tint| {
        RowModel::new(RowId(80), "Notice people with the camera")
            .look(RowLook::Flat)
            .detail("Wakes the screen. Nothing leaves the Show.")
            .dress(tile(tint))
    };
    // The tile's own alpha, far from its glyph, and how many pixels the square covers (a round of 14 takes 168 of the 2304 away; the
    // 12 the toolkit had took 124).
    let look = |h: &mut Harness, tint: Option<ValueTone>| {
        let r = row(h, &model(tint));
        let pixels = h.layer_pixels(text_layer(&r).id());
        let width = (AT.w - 2.0 * PAD_X) as usize;
        let top = ((72.0f32 - 48.0) / 2.0).round() as usize;
        let alpha = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
        let covered = (0..48usize)
            .flat_map(|y| (0..48usize).map(move |x| (x, y)))
            .filter(|&(x, y)| alpha(x, top + y) >= 20)
            .count();
        (alpha(5, top + 24), covered)
    };
    let (accent, covered) = look(&mut h, Some(ValueTone::Good));
    assert!((40..=42).contains(&accent), "16 % of 255 is 41: {accent}");
    assert!(
        (2115..=2150).contains(&covered),
        "a square of 48 with corners of 14: {covered}"
    );
    // The danger colour is as strong, a warning a little more (the plugin rows' `.row.plug.stopped` and `.slow`), the quiet one as it was.
    assert_eq!(look(&mut h, Some(ValueTone::Bad)).0, accent);
    assert!(look(&mut h, Some(ValueTone::Warn)).0 >= 50);
}

#[test]
fn a_tile_that_is_not_a_fields_the_plugins_and_a_layers_entries_keeps_its_12_round_and_its_48_of_the_tone()
 {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    let model = |tint| {
        RowModel::new(RowId(81), "Countdowns")
            .look(RowLook::Flat)
            .detail("Wakes the screen. Nothing leaves the Show.")
            .dress(Dress {
                field: false,
                ..tile(tint)
            })
    };
    let look = |h: &mut Harness, tint: Option<ValueTone>| {
        let r = row(h, &model(tint));
        let pixels = h.layer_pixels(text_layer(&r).id());
        let width = (AT.w - 2.0 * PAD_X) as usize;
        let top = ((72.0f32 - 48.0) / 2.0).round() as usize;
        let alpha = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
        let covered = (0..48usize)
            .flat_map(|y| (0..48usize).map(move |x| (x, y)))
            .filter(|&(x, y)| alpha(x, top + y) >= 20)
            .count();
        (alpha(5, top + 24), covered)
    };
    for tint in [
        Some(ValueTone::Good),
        Some(ValueTone::Bad),
        Some(ValueTone::Warn),
        None,
    ] {
        let (alpha, covered) = look(&mut h, tint);
        assert_eq!(alpha, 48, "{tint:?}");
        assert!(
            (2165..=2195).contains(&covered),
            "a square of 48 with corners of 12: {covered}"
        );
    }
}

#[test]
fn a_value_is_cut_to_two_fifths_of_a_row_but_on_a_row_with_a_tile_only_for_want_of_room() {
    use crate::lucent::tokens::ty;
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    let at = Frame::new(40.0, 100.0, 424.0, 64.0);
    let words = "10 to 15 a second";
    let whole = h.ui.fonts.measure(words, h.ui.look.ty(ty::LABEL)).width;
    // How wide the value's ink is: from its first column to its last, right of the label.
    let extent = |h: &mut Harness, dressed: bool| {
        let mut model = RowModel::new(RowId(81), "Rate")
            .look(RowLook::Flat)
            .trailing(Trailing::Value(words.into()));
        if dressed {
            model = model.dress(tile(Some(ValueTone::Good)));
        }
        let r = Row::new(&mut h.ui, at, 10, &model).unwrap();
        h.commit();
        let pixels = h.layer_pixels(text_layer(&r).id());
        let width = (at.w - 2.0 * PAD_X) as usize;
        let columns: Vec<usize> = (200..width)
            .filter(|&x| (0..64usize).any(|y| pixels[(y * width + x) * 4 + 3] > 60))
            .collect();
        (columns.last().unwrap() - columns.first().unwrap()) as f32
    };
    assert!(
        extent(&mut h, false) < 0.4 * at.w + 2.0 && extent(&mut h, false) < whole - 8.0,
        "a row of words alone cuts its value at two fifths"
    );
    assert!(
        extent(&mut h, true) >= whole - 4.0,
        "a row with a tile says all of it: its label has not asked for the room"
    );
}

#[test]
fn a_row_that_opens_something_ends_as_the_mocks_nav_row_does_a_28_chevron_16_after_its_value_and_the_value_at_the_mocks_height()
 {
    let mut h = harness();
    h.ui.look.text_scale = 1.2;
    let at = Frame::new(40.0, 100.0, 640.0, 72.0);
    let ends = |h: &mut Harness, look: RowLook, field: bool| {
        let mut model = RowModel::new(RowId(82), "Sensitivity")
            .look(look)
            .detail("How much movement counts as someone")
            .trailing(Trailing::Count("Normal".into()))
            .actionable();
        if field {
            model = model.dress(tile(None));
        }
        let r = Row::new(&mut h.ui, at, 10, &model).unwrap();
        h.commit();
        let pixels = h.layer_pixels(text_layer(&r).id());
        let (width, height) = ((at.w - 2.0 * PAD_X) as usize, 72usize);
        let ink = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3] > 100;
        // The chevron is in the last 28 columns, the value before them (right of the words).
        let chevron: Vec<(usize, usize)> = (width - 28..width)
            .flat_map(|x| (0..height).map(move |y| (x, y)))
            .filter(|&(x, y)| ink(x, y))
            .collect();
        let value: Vec<(usize, usize)> = (width - 130..width - 28)
            .flat_map(|x| (0..height).map(move |y| (x, y)))
            .filter(|&(x, y)| ink(x, y))
            .collect();
        let extent = |p: &[(usize, usize)]| {
            (
                width - p.iter().map(|q| q.0).min().unwrap(),
                width - p.iter().map(|q| q.0).max().unwrap(),
                p.iter().map(|q| q.1).min().unwrap(),
                p.iter().map(|q| q.1).max().unwrap(),
            )
        };
        (extent(&chevron), extent(&value))
    };
    // The mock's, measured on its shot: the chevron's ink is 11 wide and 20 tall, from 15 to 5 left of the row's content edge, and the
    // value ends 40 left of it with its baseline 8 under the row's middle.
    let ((from, to, top, bottom), (_, value_end, _, baseline)) = ends(&mut h, RowLook::Flat, true);
    assert!(
        (14..=16).contains(&from) && (4..=6).contains(&to),
        "{from} {to}"
    );
    assert!((18..=21).contains(&(bottom - top + 1)), "{top} {bottom}");
    assert!(
        (38..=42).contains(&value_end),
        "the value ends {value_end} from the edge"
    );
    // A page's row without a tile, a plugin's and the toolkit's other looks keep the 24 glyph and the 8, and the value where Lucent's
    // run sets it: only the rows of a page's fields (with their tile) are the mock's `nav` row.
    for (look, field) in [
        (RowLook::Card, false),
        (RowLook::Flat, false),
        (RowLook::Plain, false),
    ] {
        let ((from, _, top, bottom), (_, _, _, low)) = ends(&mut h, look, field);
        assert!(
            bottom - top < 17 && from <= 16,
            "{look:?}: {top} {bottom} {from}"
        );
        assert_eq!(
            low,
            baseline + 2,
            "{look:?}: the value of a field's row is 2 higher"
        );
    }
}
