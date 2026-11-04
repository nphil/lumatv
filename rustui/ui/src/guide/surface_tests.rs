//! The guide's surface: what it draws for a set of channels and their listings, that it reads clean in both materials at 100 and 200 percent,
//! that moving the focus along a row or down the rows makes (almost) no layer, and that hundreds of channels cost no more layers than a few.

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
        Up { _h: h, kit, engine, g: Guide::new(), s: GuideSurface::new(60), fixture: Fixture::sized(0, 0), t: NOW, clock }
    }

    fn new(mode: Mode, scale: f32, antenna: usize, internet: usize) -> Self {
        let mut up = Self::bare(mode, scale);
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
            if !moving && self.t > NOW + 1500 && self.s.layers() > 0 {
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
    // what the scene holds, of its 256.
    assert!(most <= 140, "at most {most} layers held at any step of the walk");
    assert!(up.kit.room().layers > 150, "the scene keeps its room: {}", up.kit.room().layers);
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
    assert!(held < 256 - 40);
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
