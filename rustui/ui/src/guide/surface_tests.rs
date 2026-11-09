//! The guide's surface: what it draws for a set of channels and their listings, that it reads clean in both materials at 100 and 200 percent,
//! that moving the focus along a row or down the rows makes (almost) no layer, and that hundreds of channels cost no more layers than a few.

use super::cells;
use super::fixture::Fixture;
use super::model::{Guide, HOUR, MIN, Target};
use super::surface::GuideSurface;
use crate::host::channels as ch;
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input};
use crate::kit::testkit;
use crate::kit::{Engine, Kit};
use serde_json::Value;
use ui::geometry::Frame;
use std::cell::Cell;
use std::rc::Rc;
use ui::lucent::palette::Mode;

/// 2026-11-04 04:20 UTC.
const NOW: u64 = 1_793_766_000_000;

struct Up {
    _h: ui::testing::Harness,
    kit: Kit,
    engine: Engine,
    g: Guide,
    s: GuideSurface,
    fixture: Fixture,
    t: u64,
    /// When the guide was opened.
    t0: u64,
    clock: Rc<Cell<u64>>,
}

impl Up {
    /// A guide that has not been opened.
    fn bare(mode: Mode, scale: f32) -> Self {
        let (mut h, kit) = testkit::kit_in(mode, scale);
        h.settle_atlas();
        let clock = Rc::new(Cell::new(5_000_000_000));
        let read = Rc::clone(&clock);
        kit.ui.scene.set_clock(move || read.get(), 0);
        let engine = Engine::new(&kit).expect("an engine");
        Up { _h: h, kit, engine, g: Guide::new(), s: GuideSurface::new(60), fixture: Fixture::sized(0, 0), t: NOW, t0: NOW, clock }
    }

    fn new(mode: Mode, scale: f32, antenna: usize, internet: usize) -> Self {
        Self::at(mode, scale, antenna, internet, 0)
    }

    /// A guide opened `later` minutes after [`NOW`].
    fn at(mode: Mode, scale: f32, antenna: usize, internet: usize, later: u64) -> Self {
        let mut up = Self::bare(mode, scale);
        (up.t, up.t0) = (NOW + later * MIN as u64, NOW + later * MIN as u64);
        up.fixture = Fixture::sized(antenna, internet);
        let fx = up.g.open(up.t);
        up.answer(fx);
        up.settle();
        up
    }

    /// The scene's clock is the test's: the tracks' estimates and the engine's springs are read at the time the test says.
    fn ns(&self) -> u64 {
        let ns = (self.t - NOW) * 1_000_000 + 5_000_000_000;
        self.clock.set(ns);
        ns
    }

    /// What the bridge does: every call of the guide is answered, and what the answers ask for in turn.
    fn answer(&mut self, mut fx: Vec<Effect>) {
        for _ in 0..40 {
            let mut next = Vec::new();
            for e in fx {
                if let Effect::Luna { tag, uri, payload, .. } = e
                    && let Some(reply) = self.fixture.answer(&uri, &payload, self.t)
                {
                    next.extend(self.g.update(&Input::Luna { tag, payload: reply }, self.t));
                }
            }
            if next.is_empty() {
                break;
            }
            fx = next;
        }
    }

    /// Time passes in frames of 8 ms until the guide has nothing more to ask or to draw.
    fn settle(&mut self) {
        for _ in 0..400 {
            self.t += 8;
            let fx = self.g.update(&Input::Tick, self.t);
            self.answer(fx);
            self.apply();
            let moving = self.s.busy(&self.engine, self.ns()) || self.g.due_at().is_some_and(|d| d <= self.t + 200 && d > self.t);
            if !moving && self.t > self.t0 + 1500 && self.s.layers() > 0 {
                break;
            }
        }
    }

    fn apply(&mut self) {
        for _ in 0..12 {
            let page = self.s.page(&mut self.kit);
            self.g.set_page(page);
            let ns = self.ns();
            self.s.apply(&mut self.kit, &mut self.engine, &self.g, ns).expect("the guide is drawn");
            if !self.s.pending() {
                break;
            }
        }
    }

    fn key(&mut self, name: &str) {
        self.t += 40;
        self.s.on_key(&mut self.engine);
        let fx = self.g.key(name, true, false, self.t);
        self.answer(fx);
        self.apply();
    }

    fn keys(&mut self, name: &str, n: usize) {
        for _ in 0..n {
            self.key(name);
        }
    }

    fn lint(&mut self) -> Vec<String> {
        let mut lint = Lint::new().with_safe(Frame::new(64.0, 54.0, 1792.0, 972.0));
        self.s.feed_lint(&mut self.kit, &mut lint, &self.g);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    fn geometry(&self) -> Value {
        self.s.geometry(&self.engine)
    }
}

#[test]
fn a_guide_with_listings_has_the_head_the_rows_the_strip_and_the_hints_in_a_few_dozen_layers_and_no_picture_of_text() {
    let up = Up::new(Mode::Dark, 1.0, 57, 459);
    let g = up.geometry();
    assert_eq!(g["up"], true);
    let rows = g["rows"].as_array().unwrap();
    assert!(rows.len() >= g["page"]["rows"].as_u64().unwrap() as usize, "a row for every row of the page: {rows:?}");
    assert!(rows.iter().any(|r| r.as_u64() == Some(up.g.row() as u64)), "the row in focus has layers");
    assert!(up.s.layers() <= 70, "{} layers", up.s.layers());
    assert_eq!(up.kit.ui.text_pictures, 0, "every word is glyph quads");
    assert_eq!(up.kit.ui.glyph_text_fallbacks, 0);
    let chips: Vec<&str> = g["chips"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert_eq!(chips, ["All", "Antenna", "Internet", "Now"], "the internet's chip is offered until its list says there is none");
}

#[test]
fn every_look_reads_clean_in_both_materials_at_100_and_200_percent() {
    for mode in [Mode::Dark, Mode::Light] {
        for scale in [1.0, 2.0] {
            let mut up = Up::new(mode, scale, 57, 0);
            let name = format!("{mode:?} at {scale}");
            assert_eq!(up.lint(), Vec::<String>::new(), "the grid, {name}");
            up.keys("right", 2);
            up.settle();
            assert_eq!(up.lint(), Vec::<String>::new(), "after two cells, {name}");
            up.keys("down", 3);
            up.settle();
            assert_eq!(up.lint(), Vec::<String>::new(), "three rows down, {name}");
            up.key("info");
            up.settle();
            assert!(up.geometry()["sheet"].is_object(), "the sheet is up");
            assert_eq!(up.lint(), Vec::<String>::new(), "the sheet, {name}");
            up.key("back");
            up.settle();
            up.keys("up", 24);
            up.settle();
            assert_eq!(up.g.region(), super::model::Region::Chips);
            assert_eq!(up.lint(), Vec::<String>::new(), "the chips, {name}");
        }
    }
}

#[test]
fn a_move_along_a_row_makes_at_most_a_plate_and_a_move_down_a_plate_and_a_row() {
    let mut up = Up::new(Mode::Dark, 1.0, 57, 0);
    up.keys("down", 2);
    up.settle();
    let before = up.kit.room().layers;
    up.key("right");
    let after_right = up.kit.room().layers;
    assert!(before.saturating_sub(after_right) <= 1, "a plate for the new cell and nothing else: {before} -> {after_right}");
    up.settle();
    up.keys("right", 6);
    up.settle();
    let before = up.kit.room().layers;
    up.keys("down", 4);
    up.settle();
    // a row is four layers and one is kept a row or two past the window so that a key back does not make it again
    assert!(before.abs_diff(up.kit.room().layers) <= 9, "rows come and go: {before} -> {}", up.kit.room().layers);
}

#[test]
fn hundreds_of_channels_and_days_of_listings_cost_no_more_layers_than_a_few_and_never_more_than_a_budget() {
    let mut up = Up::new(Mode::Dark, 1.0, 0, 459);
    assert_eq!(up.g.rows(), 459);
    let mut most = up.s.layers();
    for _ in 0..30 {
        up.key("chdown");
        most = most.max(up.s.layers());
    }
    up.settle();
    assert!(up.g.row() > 150, "paged a long way down: {}", up.g.row());
    for _ in 0..40 {
        up.key("right");
        up.key("down");
        most = most.max(up.s.layers());
    }
    up.settle();
    // The screen of rows being left and the one flown to (a key every 40 ms, twice a remote's repeat, leaves the spring more than a screen
    // behind) and the chrome: 130 held at most. What one frame draws is the screen's (`tests/host_budget.py guide` measures it); this is
    // what the scene holds, of its 512.
    assert!(most <= 140, "at most {most} layers held at any step of the walk");
    let held = ui::limits::MAX_DRAWABLE_NODES - up.kit.room().layers;
    assert!(held < 106, "the scene keeps its room: {held} held");
}

#[test]
fn at_rest_the_guide_asks_for_no_frame_and_a_scroll_asks_for_them_until_it_settles() {
    let mut up = Up::new(Mode::Dark, 1.0, 57, 0);
    up.settle();
    assert!(!up.s.busy(&up.engine, up.ns() + 1_000_000_000), "nothing moves at rest");
    up.keys("down", 12);
    assert!(up.s.busy(&up.engine, up.ns()), "the rows are still on their way");
    up.t += 600;
    assert!(!up.s.busy(&up.engine, up.ns()), "and then they are not");
}

#[test]
fn a_title_pinned_to_the_edge_waits_for_the_time_to_stop_when_the_cell_it_speaks_for_changes_in_a_flight() {
    let mut up = Up::new(Mode::Dark, 1.0, 57, 0);
    up.settle();
    assert_eq!(up.geometry()["leadsWaiting"], 0, "at rest every pinned title is in");
    up.keys("right", 6);
    assert!(up.geometry()["leadsWaiting"].as_u64().unwrap() > 0, "the cells fly under the edge: a pinned title of a new cell would sit on theirs");
    up.settle();
    assert_eq!(up.geometry()["leadsWaiting"], 0, "and comes in when they have stopped");
}

#[test]
fn the_progress_of_a_programme_on_the_air_is_a_line_as_long_as_it_has_run() {
    use compositor_protocol::{Content, Request};
    let mut up = Up::new(Mode::Dark, 1.0, 57, 0);
    up.settle();
    up._h.settle_atlas();
    let g = up.geometry();
    let (cells_w, row_h) = ((g["grid"]["w"].as_f64().unwrap() * 2.0) as u32, g["page"]["rowH"].as_f64().unwrap() as u32);
    let accent = up.kit.ui.look.accent();
    let mut longest = 0;
    for r in up._h.all() {
        let Request::Create { id, width, height, content: Content::Glyphs(_), .. } = r else { continue };
        if (width, height) != (cells_w, row_h) {
            continue;
        }
        let pixels = up._h.layer_pixels(id);
        for y in row_h - 12..row_h {
            let mut run = 0;
            for x in 0..width as usize {
                let at = (y as usize * width as usize + x) * 4;
                run = if pixels[at..at + 4] == [accent.0, accent.1, accent.2, 255] { run + 1 } else { 0 };
                longest = longest.max(run);
            }
        }
    }
    // one pixel is what a hairline was when the atlas had been warmed with a narrow one
    assert!(longest >= 24, "the longest line of progress in the rows is {longest} pixels");
}

#[test]
fn the_chosen_filter_chip_is_marked_and_choosing_another_moves_no_chip() {
    let mut up = Up::new(Mode::Dark, 1.0, 57, 20);
    let frames = |g: &Value| g["chips"].as_array().unwrap().iter().map(|c| (c["frame"]["x"].as_f64().unwrap(), c["frame"]["w"].as_f64().unwrap())).collect::<Vec<_>>();
    let on = |g: &Value| g["chips"].as_array().unwrap().iter().map(|c| c["on"].as_bool().unwrap()).collect::<Vec<_>>();
    let (before, marked) = (frames(&up.geometry()), on(&up.geometry()));
    assert_eq!(marked, [false, true, false, false], "Antenna is the filter that is on");
    let fx = up.g.click(Target::Chip(2), up.t);
    up.answer(fx);
    up.settle();
    let g = up.geometry();
    assert_eq!(on(&g), [false, false, true, false], "the check is on the one chosen now");
    assert_eq!(frames(&g), before, "every filter chip has the room of it, so none moves");
}

#[test]
fn the_pointer_names_what_it_is_over_and_a_click_is_a_press_that_ends_on_it() {
    let mut up = Up::new(Mode::Dark, 1.0, 57, 0);
    let g = up.geometry();
    let chip = &g["chips"][1]["frame"];
    let (cx, cy) = (chip["x"].as_f64().unwrap() as f32 + 10.0, chip["y"].as_f64().unwrap() as f32 + 10.0);
    let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
    let ns = up.ns();
    assert_eq!(up.s.pointer(&up.kit, &mut up.engine, ns, at(cx, cy, 1), &up.g), None, "a press is not a click");
    assert_eq!(up.s.pointer(&up.kit, &mut up.engine, ns, at(cx, cy, 0), &up.g), Some(Target::Chip(1)));
    // a cell of the first row, at the middle of the grid
    let grid = &g["grid"];
    let (gx, gy) = (grid["x"].as_f64().unwrap() as f32 + grid["w"].as_f64().unwrap() as f32 / 2.0, grid["y"].as_f64().unwrap() as f32 + 10.0);
    up.s.pointer(&up.kit, &mut up.engine, ns, at(gx, gy, 1), &up.g);
    match up.s.pointer(&up.kit, &mut up.engine, ns, at(gx, gy, 0), &up.g) {
        Some(Target::Cell { row, at }) => {
            assert_eq!(row, up.g.top());
            assert!(at >= up.g.view() && at < up.g.view() + up.g.page().span, "an instant of the grid: {at}");
        }
        other => panic!("a cell: {other:?}"),
    }
    // a press that began on a chip and ended elsewhere is no click
    up.s.pointer(&up.kit, &mut up.engine, ns, at(cx, cy, 1), &up.g);
    assert_eq!(up.s.pointer(&up.kit, &mut up.engine, ns, at(gx, gy, 0), &up.g), None);
}

#[test]
fn a_set_with_no_channel_says_so_and_a_service_that_does_not_answer_says_that() {
    let mut up = Up::new(Mode::Dark, 1.0, 0, 0);
    assert_eq!(up.geometry()["empty"], "No channels");
    assert_eq!(up.lint(), Vec::<String>::new());
    let mut up = Up::bare(Mode::Dark, 1.0);
    up.g.open(NOW);
    up.g.update(&Input::Luna { tag: super::model::TAG_LIST, payload: serde_json::json!({"returnValue": false, "errorText": "not running", "errorCode": -1}) }, NOW);
    up.t = NOW + 3000;
    up.apply();
    assert_eq!(up.geometry()["empty"], "The guide service did not answer");
    assert_eq!(up.lint(), Vec::<String>::new());
}

#[test]
fn leaving_gives_every_layer_back() {
    let mut up = Up::new(Mode::Dark, 1.0, 57, 0);
    let held = up.kit.room().layers;
    assert!(held < ui::limits::MAX_DRAWABLE_NODES - 40);
    let fx = up.g.close("test", up.t);
    up.answer(fx);
    for _ in 0..800 {
        up.t += 8;
        let fx = up.g.update(&Input::Tick, up.t);
        up.answer(fx);
        up.apply();
        if !up.s.is_up() {
            break;
        }
    }
    assert!(!up.s.is_up(), "gone once its life is over");
    assert_eq!(up.s.layers(), 0);
    assert!(up.kit.room().layers > held + 30, "the scene has its room back: {} -> {}", held, up.kit.room().layers);
}

#[test]
fn the_surface_and_the_layout_make_no_colour_of_their_own() {
    for (name, src) in [("surface.rs", include_str!("surface.rs")), ("layout.rs", include_str!("layout.rs"))] {
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("Rgba("), "{name} makes a colour");
    }
}

#[test]
fn time_is_asked_for_on_the_broadcasts_clock_and_shown_on_it() {
    use super::surface::{clock_text, day_text};
    let t = ch::parse_stamp("2026,11,04,23,05,00", 0).unwrap();
    assert_eq!(clock_text(t, 0), "11:05 PM");
    assert_eq!(clock_text(t, -5 * 3600), "6:05 PM");
    assert_eq!(clock_text(t + 55 * MIN, 0), "12:00 AM");
    assert_eq!(day_text(t, 0), "Wed Nov 4");
    assert_eq!(day_text(t + HOUR, 0), "Thu Nov 5");
}

/// The channels of the fixture that follow its 57 plain ones: a ticker of short programmes, a long name, no logo, no listings, a ticker.
const PLAIN: usize = 57;
const EDGES: usize = 5;

impl Up {
    /// Enough time for the rows on the screen to ask for their programmes and have the answers: the guide looks four times, 80 ms apart.
    fn look(&mut self) {
        for _ in 0..4 {
            self.t += 80;
            let fx = self.g.update(&Input::Tick, self.t);
            self.answer(fx);
            self.apply();
        }
    }

    /// `n` presses of `name`, a look after each.
    fn press(&mut self, name: &str, n: usize) {
        for _ in 0..n {
            self.key(name);
            self.look();
        }
    }

    /// CH- until the focus is on the last row, at any size (a page is a few rows fewer than the screen holds), and its programmes are in.
    fn walk_to_the_last_row(&mut self) {
        self.keys("chdown", 16);
        self.settle();
    }
}

#[test]
fn nothing_of_the_grid_is_on_a_word_and_no_word_leaves_its_cell_or_the_plate_the_grid_whatever_the_clock_the_size_the_look_and_the_place() {
    // the guide opened at the half hour, ten minutes and eleven after it (a cell that begins at the screen's edge, a minute inside it), the
    // focus walked along a row, down to the edge channels and up to the chips
    for (mode, scale, later) in [(Mode::Dark, 1.0, 10), (Mode::Dark, 1.0, 0), (Mode::Dark, 1.5, 11), (Mode::Dark, 2.0, 0), (Mode::Dark, 2.0, 9), (Mode::Light, 1.0, 11), (Mode::Light, 1.5, 0), (Mode::Light, 2.0, 10)] {
        let mut up = Up::at(mode, scale, PLAIN + EDGES, 0, later);
        let name = format!("{mode:?} at {scale}, opened {later} minutes on");
        let said = |up: &mut Up, what: &str| assert_eq!(up.lint(), Vec::<String>::new(), "{what}, {name}");
        said(&mut up, "open");
        for n in 1..=4 {
            up.press("right", 1);
            said(&mut up, &format!("{n} cells along"));
        }
        up.press("left", 6);
        said(&mut up, "back before the opening view");
        up.press("down", 2);
        said(&mut up, "two rows down");
        up.key("info");
        up.look();
        said(&mut up, "a sheet");
        up.key("back");
        up.walk_to_the_last_row();
        said(&mut up, "the last page");
        for n in 1..=3 {
            up.press("up", 1);
            up.press("right", 1);
            said(&mut up, &format!("{n} rows up the edge channels, a cell along"));
        }
        up.press("up", 60);
        assert_eq!(up.g.region(), super::model::Region::Chips);
        said(&mut up, "the chips");
    }
}

#[test]
fn a_title_is_whole_or_five_letters_and_an_ellipsis_or_not_there_so_a_cell_of_a_few_minutes_says_nothing() {
    for scale in [1.0, 2.0] {
        let mut up = Up::new(Mode::Dark, scale, PLAIN + EDGES, 0);
        up.walk_to_the_last_row();
        let g = up.geometry();
        let (gx, gw, ppm) = (g["grid"]["x"].as_f64().unwrap() as f32, g["grid"]["w"].as_f64().unwrap() as f32, g["page"]["ppm"].as_f64().unwrap() as f32);
        let words = up.s.cell_words();
        assert!(words.len() > 10, "the last page has words: {words:?}");
        for (row, text, frame) in &words {
            let kept = text.trim_end_matches('\u{2026}').chars().count();
            assert!(text == text.trim() && (!text.ends_with('\u{2026}') || kept >= cells::MIN_CUT), "{text:?} of row {row} is a fragment at {scale}");
            // the words of a cell the grid does not reach are in the run for a flight, outside it
            if frame.right() > gx && frame.x < gx + gw {
                assert!(frame.x >= gx && frame.right() <= gx + gw, "{text:?} of row {row} is inside the grid at {scale}");
            }
        }
        // the ticker's row has cells of five minutes: those are 40 px at 100 %, and no word of the row begins in one that has no room for a word
        let pad = up.kit.space(3);
        let (origin, view) = (up.g.origin(), up.g.view());
        let x_of = |t: i64| gx + ((t - view) as f64 / MIN as f64 * f64::from(ppm)).round() as f32;
        let id = up.g.channel(PLAIN).unwrap().id.clone();
        let cells: Vec<_> = up.g.spans(&id, view, view + up.g.page().span).into_iter().map(|s| (x_of(s.start), x_of(s.end))).collect();
        assert!(cells.len() > 6, "{} cells in the ticker's row, from {origin}", cells.len());
        for (a, b) in cells.iter().filter(|(a, b)| b - a < 2.0 * pad + 16.0) {
            assert!(!words.iter().any(|(row, _, f)| *row == PLAIN && f.x >= *a && f.x < *b), "a word in the cell {a}..{b} of {} px at {scale}", b - a);
        }
    }
}

#[test]
fn a_sheet_is_as_tall_as_what_it_says_and_a_row_with_no_information_has_no_dangling_separator_or_empty_line() {
    for (mode, scale) in [(Mode::Dark, 1.0), (Mode::Light, 1.0), (Mode::Dark, 2.0), (Mode::Light, 2.0)] {
        let mut up = Up::new(mode, scale, PLAIN + EDGES, 0);
        // a programme with a description, then the row of the edge channel that has no listings
        for (rows_down, what) in [(0, "a programme"), (PLAIN + 3 - 5, "no information")] {
            if rows_down > 0 {
                up.press("down", rows_down);
            }
            up.key("info");
            up.look();
            let g = up.geometry();
            let (sheet, watch) = (&g["sheet"], &g["watch"]);
            let (bottom, pad) = (sheet["y"].as_f64().unwrap() + sheet["h"].as_f64().unwrap(), f64::from(up.kit.space(6)));
            assert_eq!(watch["y"].as_f64().unwrap() + watch["h"].as_f64().unwrap() + pad, bottom, "nothing under the button: {what}, {mode:?} at {scale}");
            let words = up.s.sheet_words();
            assert!(words.iter().all(|w| !w.trim_end().ends_with('\u{b7}') && !w.is_empty()), "{words:?}: {what}, {mode:?} at {scale}");
            if rows_down > 0 {
                assert_eq!(words, ["62-1 WWWWWWWWWWWW-HD", "No information"], "a row that has none says so and nothing else");
            }
            up.key("back");
            up.look();
        }
    }
}

#[test]
fn a_channel_with_no_logo_says_its_call_sign_on_its_tile_and_a_name_of_words_its_initials() {
    use super::surface::monogram;
    assert_eq!(monogram("WSB-HD"), "WSB");
    assert_eq!(monogram("WAGA-HD"), "WAGA");
    assert_eq!(monogram("ION"), "ION");
    assert_eq!(monogram("Dabl"), "DABL");
    assert_eq!(monogram("WSVP-HD Southern Valley Public Television"), "WSVP");
    assert_eq!(monogram("Pluto TV Classic Sitcoms"), "PTCS");
    assert_eq!(monogram("Pluto TV Classic Sitcoms And More"), "PTCS");
    assert_eq!(monogram("WWWWWWWWWWWW-HD"), "WWWW");
    assert_eq!(monogram(""), "");
    assert_eq!(monogram("-"), "");
    let up = Up::new(Mode::Dark, 1.0, PLAIN + EDGES, 0);
    let rail = up.s.rail_words();
    assert!(rail.iter().any(|(row, w)| *row == 0 && w == "KAR"), "the first channel has no logo in this set: {rail:?}");
    assert!(rail.iter().any(|(row, w)| *row == 0 && w == "KAR-HD"), "and its name beside the tile: {rail:?}");
}
