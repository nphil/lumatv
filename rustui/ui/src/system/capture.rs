//! The capture indicator (`panels/CaptureMark.qml` of the QML shell, `docs/research/capture-result.md`): a red dot in the top left corner
//! for as long as the compositor says a recording runs, and a short note when a capture ends. The compositor records only while this
//! says the dot is on the screen (`state.capture.dot`), so the dot is reported from what the surface drew, never from the model.
//!
//! The dot is a filled circle in the palette's live red inside a dark ring, so it reads over any picture and the circle's shape says
//! "recording" without the colour. It dims every other second (a step, not a fade) so a frozen picture is not mistaken for a stopped
//! recording and a bright dot is not left burning one place on the panel; with reduced motion it stands still. Nothing here takes a key.

use super::parts::{HIDDEN, Sheet};
use crate::kit::Kit;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::screen::{Effect, Input};
use crate::kit::text::{Text, role_style, width_of};
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::TypeRole;
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::ShapeLayer;

/// How long a note stays.
const NOTE_MS: u64 = 2600;
/// The dot dims and brightens this often.
const BLINK_MS: u64 = 1000;
const RING: f32 = 36.0;
const DOT: f32 = 22.0;
/// The dot's opacity in its dim half.
const DIM: f32 = 0.45;

/// What the surface draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub recording: bool,
    pub bright: bool,
    pub note: String,
}

#[derive(Debug)]
pub struct Capture {
    recording: bool,
    bright: bool,
    reduced: bool,
    /// What the shell had seen when it began: captures from before it started are not news.
    seen_n: Option<i64>,
    seen_at: f64,
    note: String,
    note_until: Option<u64>,
    blink_at: Option<u64>,
}

impl Capture {
    pub fn new(reduced: bool) -> Self {
        Self { recording: false, bright: true, reduced, seen_n: None, seen_at: -1.0, note: String::new(), note_until: None, blink_at: None }
    }

    /// Reduced motion: the dot stands still.
    pub fn set_reduced(&mut self, reduced: bool) {
        if self.reduced != reduced {
            self.reduced = reduced;
            self.bright = true;
            self.blink_at = None;
        }
    }

    /// Something of this is on the screen, or is about to be taken off it.
    pub fn live(&self) -> bool {
        self.recording || !self.note.is_empty()
    }

    pub fn recording(&self) -> bool {
        self.recording
    }

    pub fn view(&self) -> View {
        View { recording: self.recording, bright: self.bright, note: self.note.clone() }
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Topic { topic, data } if topic == "capture" => return self.take(data, now),
            Input::Tick => return self.tick(now),
            _ => {}
        }
        Vec::new()
    }

    fn take(&mut self, t: &Value, now: u64) -> Vec<Effect> {
        let recording = t.get("recording").and_then(Value::as_bool) == Some(true);
        let mut changed = recording != self.recording;
        if recording != self.recording {
            self.recording = recording;
            self.bright = true;
            self.blink_at = (recording && !self.reduced).then_some(now + BLINK_MS);
        }
        if let Some(n) = t.get("n").and_then(Value::as_i64) {
            let last = t.get("last").filter(|l| l.is_object());
            let at = last.and_then(|l| l.get("at")).and_then(Value::as_f64).unwrap_or(0.0);
            match self.seen_n {
                None => {
                    self.seen_n = Some(n);
                    self.seen_at = at;
                }
                Some(seen) => {
                    let mut words = "";
                    if n > seen {
                        words = "Screenshot saved";
                    } else if let Some(l) = last
                        && (at - self.seen_at).abs() > f64::EPSILON
                    {
                        let ok = l.get("ok").and_then(Value::as_bool);
                        match l.get("kind").and_then(Value::as_str) {
                            Some("recording") => words = if ok == Some(true) { "Recording saved" } else { "Recording stopped" },
                            Some("shot") if ok == Some(false) => words = "Screenshot failed",
                            _ => {}
                        }
                    }
                    self.seen_n = Some(n);
                    self.seen_at = at;
                    if !words.is_empty() {
                        self.note = words.to_string();
                        self.note_until = Some(now + NOTE_MS);
                        changed = true;
                    }
                }
            }
        }
        if changed { vec![Effect::Redraw] } else { Vec::new() }
    }

    fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut redraw = false;
        if self.note_until.is_some_and(|t| now >= t) {
            self.note_until = None;
            self.note.clear();
            redraw = true;
        }
        if self.recording && !self.reduced && self.blink_at.is_some_and(|t| now >= t) {
            self.bright = !self.bright;
            self.blink_at = Some(now + BLINK_MS);
            redraw = true;
        }
        if redraw { vec![Effect::Redraw] } else { Vec::new() }
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.note_until, if self.recording { self.blink_at } else { None }].into_iter().flatten().min()
    }

    pub fn state(&self, dot: bool) -> Value {
        json!({"dot": dot, "recording": self.recording, "note": self.note})
    }
}

/// The ring and the dot, made once and shown while a recording runs, and the note's pill, made as it comes and dropped when gone.
pub struct CaptureSurface {
    z: i32,
    ring: Option<(ShapeLayer, ShapeLayer)>,
    generation: u64,
    note: Option<Note>,
    leaving: Vec<(Sheet, u64)>,
    dot_up: bool,
    over_video: bool,
}

struct Note {
    text: String,
    sheet: Sheet,
    frame: Frame,
    paint: Rgba,
    word: Frame,
    style: ui::components::Style,
    /// Whether the ring is beside it (the note is built for one place).
    beside: bool,
    over_video: bool,
}

impl CaptureSurface {
    pub fn new(z: i32) -> Self {
        Self { z, ring: None, generation: 0, note: None, leaving: Vec::new(), dot_up: false, over_video: false }
    }

    /// Whether the dot is on the screen: what the compositor's recording waits for.
    pub fn dot_up(&self) -> bool {
        self.dot_up
    }

    fn build_ring(&mut self, kit: &mut Kit) {
        let live = kit.ui.look.colour(PaletteRole::Live);
        let ring = plate::shape(&kit.ui, Frame::new(0.0, 0.0, RING, RING), self.z, plate::solid(Rgba(0, 0, 0, 0xE6), RING / 2.0));
        ring.set(HIDDEN);
        let dot = plate::shape(&kit.ui, Frame::new(0.0, 0.0, DOT, DOT), self.z + 1, plate::solid(live, DOT / 2.0));
        dot.set(HIDDEN);
        self.ring = Some((ring, dot));
        self.generation = kit.generation();
    }

    fn build_note(kit: &mut Kit, z: i32, text: &str, beside: bool, over_video: bool) -> std::io::Result<Note> {
        let inset = kit.space(6);
        let style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Left);
        let w = width_of(kit, style, text);
        let line = kit.line(TypeRole::Label);
        let (pw, ph) = (w + 2.0 * kit.space(5), line + 2.0 * kit.space(2));
        let x = if beside { inset + RING + kit.space(3) } else { inset };
        let y = if ph < RING && beside { inset + ((RING - ph) / 2.0).round() } else { inset };
        let frame = Frame::new(x, y, pw, ph);
        let mut sheet = Sheet::new();
        sheet.pane(Pane::new(kit, frame, z, PaneKind::Pill, over_video));
        let word = Frame::new(x + kit.space(5), y + kit.space(2), w, line);
        sheet.text(Text::new(kit, word, z + 1, style, text)?);
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        Ok(Note { text: text.to_string(), sheet, frame, paint, word, style, beside, over_video })
    }

    /// Draws `view`. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, view: &View, over_video: bool, now_ms: u64) -> std::io::Result<bool> {
        if self.ring.is_some() && self.generation != kit.generation() {
            self.ring = None;
        }
        if view.recording && self.ring.is_none() {
            self.build_ring(kit);
        }
        self.over_video = over_video;
        let inset = kit.space(6);
        if let Some((ring, dot)) = &self.ring {
            if view.recording {
                // on the frame the compositor asked for it: it records only once this is up
                ring.set(Props::at(inset, inset));
                let offset = ((RING - DOT) / 2.0).round();
                dot.set(Props { opacity: if view.bright { 1.0 } else { DIM }, ..Props::at(inset + offset, inset + offset) });
                self.dot_up = true;
            } else {
                ring.set(HIDDEN);
                dot.set(HIDDEN);
                self.dot_up = false;
            }
        }
        let stale = self.note.as_ref().is_some_and(|n| {
            n.text != view.note || n.beside != view.recording || n.over_video != over_video
        });
        if stale && let Some(mut gone) = self.note.take() {
            gone.sheet.hide(kit, false);
            self.leaving.push((gone.sheet, now_ms + u64::from(kit.beat_ms(crate::kit::motion::Beat::Layer)) + 40));
        }
        if self.note.is_none() && !view.note.is_empty() {
            let mut n = Self::build_note(kit, self.z + 2, &view.note, view.recording, over_video)?;
            n.sheet.show(kit, false);
            self.note = Some(n);
        }
        Ok(self.dot_up || self.note.is_some() || !self.leaving.is_empty())
    }

    pub fn tick(&mut self, now_ms: u64) -> bool {
        let n = self.leaving.len();
        self.leaving.retain(|(_, until)| *until > now_ms);
        n != self.leaving.len()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving.iter().map(|(_, until)| *until).min()
    }

    pub fn geometry(&self, kit: &Kit) -> Value {
        let inset = kit.space(6);
        json!({
            "dot": self.dot_up.then(|| json!({"x": inset, "y": inset, "w": RING, "h": RING})),
            "note": self.note.as_ref().map(|n| json!({"x": n.frame.x, "y": n.frame.y, "w": n.frame.w, "h": n.frame.h, "text": n.text})),
        })
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        if let Some(n) = &self.note {
            let panel = lint.panel("capture note", n.frame, n.paint, false);
            lint.word(kit, "capture note words", &n.text, n.word, n.style, Some(panel));
        }
    }

    pub fn restyle(&mut self) {
        self.ring = None;
        self.note = None;
        self.leaving.clear();
        self.dot_up = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(recording: bool, n: i64, last: Value) -> Input {
        Input::Topic { topic: "capture".into(), data: json!({"recording": recording, "stage": "idle", "n": n, "last": last}) }
    }

    #[test]
    fn a_recording_shows_the_dot_and_it_dims_every_other_second_unless_motion_is_reduced() {
        let mut c = Capture::new(false);
        assert!(!c.live());
        assert_eq!(c.update(&topic(true, 0, Value::Null), 1000), vec![Effect::Redraw]);
        assert!(c.live() && c.view().bright);
        assert_eq!(c.due_at(), Some(2000));
        c.update(&Input::Tick, 2000);
        assert!(!c.view().bright, "dim for one second");
        c.update(&Input::Tick, 3000);
        assert!(c.view().bright);
        c.update(&topic(false, 0, Value::Null), 3500);
        assert!(!c.live() && c.due_at().is_none(), "nothing runs once it has ended");

        let mut still = Capture::new(true);
        still.update(&topic(true, 0, Value::Null), 1000);
        assert!(still.due_at().is_none(), "reduced motion: no blink, no timer");
        still.update(&Input::Tick, 9000);
        assert!(still.view().bright);
    }

    #[test]
    fn a_capture_that_ends_is_a_note_for_two_and_a_half_seconds_and_old_ones_are_not_news() {
        let mut c = Capture::new(false);
        // the first topic only says what there already was
        c.update(&topic(false, 3, json!({"kind": "shot", "ok": true, "at": 10.0})), 0);
        assert_eq!(c.view().note, "");
        c.update(&topic(false, 4, json!({"kind": "shot", "ok": true, "at": 20.0})), 100);
        assert_eq!(c.view().note, "Screenshot saved");
        assert_eq!(c.due_at(), Some(2700));
        c.update(&Input::Tick, 2700);
        assert_eq!(c.view().note, "");
        c.update(&topic(false, 4, json!({"kind": "recording", "ok": true, "at": 30.0})), 5000);
        assert_eq!(c.view().note, "Recording saved");
        c.update(&topic(false, 4, json!({"kind": "recording", "ok": false, "at": 40.0})), 9000);
        assert_eq!(c.view().note, "Recording stopped");
        c.update(&topic(false, 4, json!({"kind": "shot", "ok": false, "at": 50.0})), 12000);
        assert_eq!(c.view().note, "Screenshot failed");
        // the same record again is not a second note
        c.update(&Input::Tick, 20000);
        c.update(&topic(false, 4, json!({"kind": "shot", "ok": false, "at": 50.0})), 20001);
        assert_eq!(c.view().note, "");
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;
    use crate::kit::testkit;
    use ui::lucent::palette::Mode;

    #[test]
    fn the_dot_is_up_exactly_while_it_is_drawn_and_the_note_beside_it_reads_clean_in_both_materials_and_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut s = CaptureSurface::new(1900);
                let none = View::default();
                assert!(!s.apply(&mut kit, &none, false, 0).unwrap());
                assert!(!s.dot_up(), "nothing recording: no dot, and so no recording may start");
                let rec = View { recording: true, bright: true, note: String::new() };
                assert!(s.apply(&mut kit, &rec, false, 10).unwrap());
                assert!(s.dot_up());
                let dim = View { bright: false, ..rec.clone() };
                s.apply(&mut kit, &dim, false, 1010).unwrap();
                assert!(s.dot_up(), "dim is still on the screen");
                let noted = View { note: "Recording saved".into(), ..rec.clone() };
                s.apply(&mut kit, &noted, true, 2000).unwrap();
                let g = s.geometry(&kit);
                let (dot, note) = (&g["dot"], &g["note"]);
                assert!(dot["x"].as_f64().is_some(), "{g}");
                assert!(note["x"].as_f64().unwrap() >= dot["x"].as_f64().unwrap() + dot["w"].as_f64().unwrap(), "the note sits beside the dot: {g}");
                let mut lint = Lint::new();
                s.feed_lint(&mut kit, &mut lint);
                let found: Vec<String> = lint.findings().iter().map(ToString::to_string).collect();
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                s.apply(&mut kit, &none, false, 3000).unwrap();
                assert!(!s.dot_up(), "the recording ended: the dot is off the screen the same frame");
            }
        }
    }
}
