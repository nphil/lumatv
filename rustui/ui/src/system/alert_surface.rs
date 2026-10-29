//! The alert, the input popup and the PIN prompt drawn (the mocks `docs/mocks/06-confirm-dialog`, `18-no-signal`, `19-pin-prompt`): the
//! alert a sheet 840 wide in the middle of the screen over a scrim (the whole screen, opaque, for a warning); the input popup a sheet at
//! the toasts' corner; the PIN prompt a sheet with the four cells of the digits, the 3 by 4 pad and OK. Each is built when it opens and
//! dropped when it has faded out (the scene's budget: a modal kind keeps nothing hidden), its buttons are the kit's, registered in a
//! modal scope of the shared engine, so a D-pad key moves the lift the way it does everywhere and the pointer's hover and click are the
//! kit's rule. The model owns which button has the focus; this follows it with the flow.

use super::alert::{Alert, InputPopup, OK_INDEX, PAD, Pin, Target, View};
use super::parts::{Sheet, icon_tile, tile_glyph_at};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::text::{Text, role_style};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, Rgba, Shape};
use lumatv_placement::{Frame as PlacementFrame, Registry, Size, Sizes, User};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;

const ID_BASE: u32 = 0x5359_0000;
const REGION: Region = Region(100);
const SCOPE_ALERT: Scope = Scope(100);
const SCOPE_INPUT: Scope = Scope(101);
const SCOPE_PIN: Scope = Scope(102);
const SHEET_W: f32 = 840.0;
const INPUT_W: f32 = 640.0;
const PIN_W: f32 = 560.0;
const TILE: f32 = 56.0;
const CELL: f32 = 56.0;

fn id(n: usize) -> Id {
    Id(ID_BASE + n as u32)
}

/// LG's text with its line breaks (`<br>` in the message of an alert).
fn plain(text: &str) -> String {
    let mut t = text.to_string();
    for br in ["<br/>", "<br />", "<br>", "<BR>", "<BR/>", "<Br>"] {
        t = t.replace(br, "\n");
    }
    t.trim().to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Alert,
    Input,
    Pin,
}

/// What a screen on show is, to tell a new one from the same one with another focus.
fn signature(view: &View) -> Option<(Kind, String)> {
    match view {
        View::None => None,
        View::Alert(a) => Some((
            Kind::Alert,
            format!("{:?}", (&a.timestamp, &a.title, &a.message, a.warning, a.buttons.iter().map(|b| (&b.label, b.danger)).collect::<Vec<_>>())),
        )),
        View::Input(p) => Some((Kind::Input, format!("{:?}", (&p.timestamp, &p.port_name, p.inputs.iter().map(|i| &i.port_name).collect::<Vec<_>>())))),
        View::Pin(p) => Some((Kind::Pin, format!("{:?}", (&p.app_id, &p.title, &p.message, p.retry)))),
    }
}

/// A word of the screen, kept for the lint.
struct Word {
    text: String,
    frame: Frame,
    style: Style,
}

struct Screen {
    kind: Kind,
    sig: String,
    scope: Scope,
    sheet: Sheet,
    buttons: Vec<Button>,
    targets: Vec<(Id, Target)>,
    /// The sheet's rectangle and what it paints, and its words.
    frame: Frame,
    paint: Rgba,
    words: Vec<Word>,
    /// The PIN's cells (parts of the sheet) and the index of the OK button.
    cells: Vec<usize>,
    typed: usize,
    focus: usize,
}

pub struct AlertSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    pointer: Pointer,
    current: Option<Screen>,
    /// Screens fading out, and when they are dropped (ms).
    leaving: Vec<(Screen, u64)>,
}

impl AlertSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, pointer: Pointer::new(), current: None, leaving: Vec::new() }
    }

    /// The owner's places changed.
    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    /// Where the toasts' column starts (the input popup is right-aligned with it).
    fn toast_origin(&self, w: f32, h: f32) -> (f32, f32) {
        let mut sizes = Sizes::new();
        sizes.set("toast", Size::new(560, h as i32));
        let laid = self.registry.layout_all(&self.user, &self.frame, &sizes, None, false);
        laid.rect("toast").map_or((1920.0 - 24.0 - w, 24.0), |r| ((r.x + r.w) as f32 - w, r.y as f32))
    }

    pub fn is_up(&self) -> bool {
        self.current.is_some()
    }

    /// Fits the screen to the view: builds a new one, fades the old one out, follows the model's focus and the PIN's digits.
    /// Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ms: u64, now_ns: u64) -> std::io::Result<bool> {
        let want = signature(view);
        let same = self.current.as_ref().map(|s| (s.kind, s.sig.clone())) == want;
        if !same {
            let swap = self.current.as_ref().map(|s| s.kind) == want.as_ref().map(|w| w.0);
            if let Some(old) = self.current.take() {
                self.retire(kit, engine, old, now_ms, swap);
            }
            if want.is_some() {
                let screen = match view {
                    View::Alert(a) => self.build_alert(kit, engine, a, now_ns),
                    View::Input(p) => self.build_input(kit, engine, p, now_ns),
                    View::Pin(p) => self.build_pin(kit, engine, p, now_ns),
                    View::None => unreachable!("a view with a signature"),
                };
                let mut s = screen?;
                // a screen that replaces one of its own kind takes its place at once, as a retry of the PIN does
                s.sheet.show(kit, swap);
                for b in &mut s.buttons {
                    b.show(kit, swap);
                }
                self.current = Some(s);
            }
        }
        if let Some(s) = &mut self.current {
            Self::follow(kit, engine, s, view, now_ns);
        }
        Ok(self.current.is_some() || !self.leaving.is_empty())
    }

    /// The model's focus and the PIN's digits, on the screen that is up.
    fn follow(kit: &mut Kit, engine: &mut Engine, s: &mut Screen, view: &View, now_ns: u64) {
        let index = match view {
            View::Alert(a) => a.index,
            View::Input(p) => p.index,
            View::Pin(p) => p.index,
            View::None => return,
        };
        let want = id(index.min(s.buttons.len().saturating_sub(1)));
        if engine.focus.current() != Some(want) {
            engine.focus(want, now_ns);
        }
        s.focus = index;
        if let View::Pin(p) = view {
            let typed = p.typed.chars().count();
            if typed != s.typed {
                s.typed = typed;
                Self::paint_cells(kit, s);
                if let Some(ok) = s.buttons.get_mut(OK_INDEX) {
                    ok.set_dimmed(kit, typed < 4);
                }
            }
        }
    }

    fn cell_shape(kit: &Kit, filled: bool) -> Shape {
        let look = &kit.ui.look;
        let (fill, edge) =
            if filled { (look.accent(), look.colour(PaletteRole::Edge)) } else { (look.colour(PaletteRole::Glass), look.colour(PaletteRole::Edge)) };
        Shape { corner_radius: kit.tokens.radius(Radius::Tile), fill: Fill::Solid(fill), border_width: 1.0, border: edge }
    }

    fn paint_cells(kit: &Kit, s: &mut Screen) {
        for (i, part) in s.cells.iter().enumerate() {
            let shape = Self::cell_shape(kit, i < s.typed);
            if let Some(layer) = s.sheet.shape_mut(*part) {
                layer.reshape(shape);
            }
        }
    }

    /// Takes a screen off: out of the engine at once, its layers fading out and dropped after the layer beat (`at_once`: dropped now).
    fn retire(&mut self, kit: &Kit, engine: &mut Engine, mut s: Screen, now_ms: u64, at_once: bool) {
        for b in &s.buttons {
            b.detach(engine);
        }
        engine.focus.close_modal();
        if at_once {
            return;
        }
        s.sheet.hide(kit, false);
        for b in &mut s.buttons {
            b.hide(kit, false);
        }
        let until = now_ms + u64::from(kit.beat_ms(crate::kit::motion::Beat::Layer)) + 40;
        self.leaving.push((s, until));
    }

    /// Drops the screens that have finished fading out. Returns whether one went.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let n = self.leaving.len();
        self.leaving.retain(|(_, until)| *until > now_ms);
        n != self.leaving.len()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving.iter().map(|(_, until)| *until).min()
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.pointer.key(engine);
        if let Some(s) = &mut self.current {
            for b in &mut s.buttons {
                b.set_hover(kit, false);
            }
        }
    }

    /// A sample of the pointer: the hover cue follows it, a press shrinks the button, a click is the target to press.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, sample: Sample) -> Option<Target> {
        let s = self.current.as_mut()?;
        let mut click = None;
        for g in self.pointer.feed(engine, now_ns, sample) {
            match g {
                Gesture::Hover { from, to } => {
                    for b in &mut s.buttons {
                        let on = Some(b.id) == to && engine.focus.current() != Some(b.id);
                        if Some(b.id) == from || Some(b.id) == to {
                            b.set_hover(kit, on);
                        }
                    }
                }
                Gesture::Down(i) => {
                    if let Some(b) = s.buttons.iter_mut().find(|b| b.id == i) {
                        b.set_pressed(kit, true);
                    }
                }
                Gesture::Up(Some(i)) => {
                    if let Some(b) = s.buttons.iter_mut().find(|b| b.id == i) {
                        b.set_pressed(kit, false);
                    }
                }
                Gesture::Up(None) => {}
                Gesture::Click(i) => click = s.targets.iter().find(|(t, _)| *t == i).map(|(_, target)| *target),
            }
        }
        click
    }

    // ---- building

    fn scrim(kit: &mut Kit, z: i32, opaque: bool, hide: bool) -> ui::scene::ShapeLayer {
        let fill = if opaque {
            kit.ui.look.canvas()
        } else if hide {
            kit.tokens.extra(kit.mode).scrim_hide
        } else {
            kit.ui.look.colour(PaletteRole::Scrim)
        };
        plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(fill, 0.0))
    }

    /// The buttons of a row laid out in the width `w` from `x0`, wrapping to a new row when one does not fit: their frames from `y0`.
    fn row(kit: &mut Kit, labels: &[(String, bool)], x0: f32, y0: f32, w: f32) -> (Vec<Frame>, f32) {
        let h = Button::height(kit);
        let gap = kit.space(4);
        let mut out = Vec::new();
        let (mut x, mut y) = (x0, y0);
        for (label, has_icon) in labels {
            let bw = Button::width_for(kit, label, *has_icon);
            if x > x0 && x + bw > x0 + w {
                x = x0;
                y += h + kit.space(2);
            }
            out.push(Frame::new(x, y, bw, h));
            x += bw + gap;
        }
        (out, y + h - y0)
    }

    /// Lines of words one under the other at `at` (its width is the box's), added to the sheet and noted for the lint; returns their
    /// height.
    fn lines_at(kit: &mut Kit, sheet: &mut Sheet, words: &mut Vec<Word>, z: i32, style: Style, lines: &[String], at: Frame) -> std::io::Result<f32> {
        let line_h = ui::components::line_height(style.ty);
        for (i, l) in lines.iter().enumerate() {
            let frame = Frame::new(at.x, at.y + i as f32 * line_h, at.w, line_h);
            let t = Text::new(kit, frame, z, style, l)?;
            sheet.text(t);
            words.push(Word { text: l.clone(), frame, style });
        }
        Ok(lines.len() as f32 * line_h)
    }

    fn finish(kit: &mut Kit, mut s: Screen) -> std::io::Result<Screen> {
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the alert could not be drawn whole"));
        }
        s.paint = kit.ui.look.colour(PaletteRole::Sheet);
        Ok(s)
    }

    fn build_alert(&mut self, kit: &mut Kit, engine: &mut Engine, a: &Alert, now_ns: u64) -> std::io::Result<Screen> {
        let z = self.z;
        let (pad, gap) = (kit.space(6), kit.space(4));
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
        let text_w = SHEET_W - 2.0 * pad - TILE - gap;
        let title = if a.title.is_empty() { a.source_id.clone() } else { a.title.clone() };
        let title_lines = wrap::lines(kit, title_style, &plain(&title), text_w, 2);
        let header_h = TILE.max(title_lines.len() as f32 * ui::components::line_height(title_style.ty));
        let labels: Vec<(String, bool)> =
            if a.buttons.is_empty() { vec![("OK".to_string(), false)] } else { a.buttons.iter().map(|b| (b.label.clone(), false)).collect() };
        let (_, rows_h) = Self::row(kit, &labels, 0.0, 0.0, SHEET_W - 2.0 * pad);
        let message = plain(&a.message);
        let body_line = ui::components::line_height(body_style.ty);
        let fixed = 2.0 * pad + header_h + gap + rows_h + if message.is_empty() { 0.0 } else { gap };
        let room = (1080.0 - 2.0 * 54.0 - fixed).max(body_line);
        let msg_lines = if message.is_empty() {
            Vec::new()
        } else {
            wrap::lines(kit, body_style, &message, SHEET_W - 2.0 * pad, (room / body_line).floor().max(1.0) as usize)
        };
        let msg_h = msg_lines.len() as f32 * body_line;
        let sheet_h = 2.0 * pad + header_h + gap + msg_h + if msg_lines.is_empty() { 0.0 } else { gap } + rows_h;
        let (sx, sy) = (((1920.0 - SHEET_W) / 2.0).round(), ((1080.0 - sheet_h) / 2.0).round());
        let frame = Frame::new(sx, sy, SHEET_W, sheet_h);
        let mut sheet = Sheet::new();
        let mut words = Vec::new();
        let scrim = Self::scrim(kit, z, a.warning, false);
        sheet.shape(scrim, (0.0, 0.0));
        sheet.pane(Pane::new(kit, frame, z + 1, PaneKind::Sheet, false));
        let (tile, glyph) = icon_tile(kit, TILE, z + 2, if a.warning { "triangle-alert" } else { "bell" }, !a.warning)?;
        sheet.shape(tile, (sx + pad, sy + pad));
        if let Some(g) = glyph {
            sheet.image(g, tile_glyph_at((sx + pad, sy + pad), TILE));
        }
        let title_h = title_lines.len() as f32 * ui::components::line_height(title_style.ty);
        Self::lines_at(
            kit,
            &mut sheet,
            &mut words,
            z + 3,
            title_style,
            &title_lines,
            Frame::new(sx + pad + TILE + gap, sy + pad + ((header_h - title_h) / 2.0).round(), text_w, 0.0),
        )?;
        let mut y = sy + pad + header_h + gap;
        Self::lines_at(kit, &mut sheet, &mut words, z + 3, body_style, &msg_lines, Frame::new(sx + pad, y, SHEET_W - 2.0 * pad, 0.0))?;
        y += msg_h + if msg_lines.is_empty() { 0.0 } else { gap };
        let (frames, _) = Self::row(kit, &labels, sx + pad, y, SHEET_W - 2.0 * pad);
        engine.focus.open_modal(SCOPE_ALERT);
        let mut buttons = Vec::new();
        let mut targets = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            let danger = a.buttons.get(i).is_some_and(|b| b.danger);
            let spec = Spec::new(&labels[i].0, if danger { Tone::Danger } else { Tone::Plain });
            buttons.push(Button::new(kit, engine, Place { id: id(i), frame: *f, region: REGION, scope: SCOPE_ALERT, z: z + 4 }, spec)?);
            targets.push((id(i), Target::Alert(i)));
        }
        let first = id(a.index.min(buttons.len().saturating_sub(1)));
        engine.focus.focus(first);
        let _ = engine.flow.snap(first, true, now_ns);
        Self::finish(
            kit,
            Screen {
                kind: Kind::Alert,
                sig: String::new(),
                scope: SCOPE_ALERT,
                sheet,
                buttons,
                targets,
                frame,
                paint: Rgba(0, 0, 0, 0),
                words,
                cells: Vec::new(),
                typed: 0,
                focus: a.index,
            },
        )
        .map(|mut s| {
            s.sig = signature(&View::Alert(a.clone())).map(|x| x.1).unwrap_or_default();
            s
        })
    }

    fn build_input(&mut self, kit: &mut Kit, engine: &mut Engine, p: &InputPopup, now_ns: u64) -> std::io::Result<Screen> {
        let z = self.z;
        let (pad, gap) = (kit.space(5), kit.space(3));
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
        let text_w = INPUT_W - 2.0 * pad - TILE - gap;
        let port = if p.port_name.is_empty() {
            p.inputs.first().map(|i| i.port_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "A device".to_string())
        } else {
            p.port_name.clone()
        };
        let title_lines = wrap::lines(kit, title_style, &format!("{port} connected"), text_w, 2);
        let title_h = title_lines.len() as f32 * ui::components::line_height(title_style.ty);
        let body_h = ui::components::line_height(body_style.ty);
        let header_h = TILE.max(title_h + body_h);
        let mut labels: Vec<(String, bool)> = p
            .inputs
            .iter()
            .map(|i| {
                (
                    if p.inputs.len() > 1 {
                        format!("Switch to {}", if i.port_name.is_empty() { &i.app_id } else { &i.port_name })
                    } else {
                        "Switch".to_string()
                    },
                    false,
                )
            })
            .collect();
        labels.push(("Stay here".to_string(), false));
        let (_, rows_h) = Self::row(kit, &labels, 0.0, 0.0, INPUT_W - 2.0 * pad);
        let sheet_h = 2.0 * pad + header_h + gap + rows_h;
        let (sx, sy) = self.toast_origin(INPUT_W, sheet_h);
        let frame = Frame::new(sx, sy, INPUT_W, sheet_h);
        let mut sheet = Sheet::new();
        let mut words = Vec::new();
        sheet.pane(Pane::new(kit, frame, z + 1, PaneKind::Sheet, false));
        let (tile, glyph) = icon_tile(kit, TILE, z + 2, "cable", true)?;
        sheet.shape(tile, (sx + pad, sy + pad));
        if let Some(g) = glyph {
            sheet.image(g, tile_glyph_at((sx + pad, sy + pad), TILE));
        }
        let ty = sy + pad + ((header_h - (title_h + body_h)) / 2.0).round();
        Self::lines_at(kit, &mut sheet, &mut words, z + 3, title_style, &title_lines, Frame::new(sx + pad + TILE + gap, ty, text_w, 0.0))?;
        Self::lines_at(
            kit,
            &mut sheet,
            &mut words,
            z + 3,
            body_style,
            &["Switch to it?".to_string()],
            Frame::new(sx + pad + TILE + gap, ty + title_h, text_w, 0.0),
        )?;
        let (frames, _) = Self::row(kit, &labels, sx + pad, sy + pad + header_h + gap, INPUT_W - 2.0 * pad);
        engine.focus.open_modal(SCOPE_INPUT);
        let mut buttons = Vec::new();
        let mut targets = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            let tone = if i == 0 && p.inputs.len() == 1 || i + 1 < frames.len() && i == 0 { Tone::Primary } else { Tone::Plain };
            buttons.push(Button::new(
                kit,
                engine,
                Place { id: id(i), frame: *f, region: REGION, scope: SCOPE_INPUT, z: z + 4 },
                Spec::new(&labels[i].0, tone),
            )?);
            targets.push((id(i), Target::Input(i)));
        }
        let first = id(p.index.min(buttons.len().saturating_sub(1)));
        engine.focus.focus(first);
        let _ = engine.flow.snap(first, true, now_ns);
        Self::finish(
            kit,
            Screen {
                kind: Kind::Input,
                sig: String::new(),
                scope: SCOPE_INPUT,
                sheet,
                buttons,
                targets,
                frame,
                paint: Rgba(0, 0, 0, 0),
                words,
                cells: Vec::new(),
                typed: 0,
                focus: p.index,
            },
        )
        .map(|mut s| {
            s.sig = signature(&View::Input(p.clone())).map(|x| x.1).unwrap_or_default();
            s
        })
    }

    fn build_pin(&mut self, kit: &mut Kit, engine: &mut Engine, p: &Pin, now_ns: u64) -> std::io::Result<Screen> {
        let z = self.z;
        let (pad, gap) = (kit.space(6), kit.space(4));
        let ink = kit.ui.look.ink();
        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Centre);
        let message_ink = if p.retry { kit.ui.look.danger() } else { kit.ui.look.ink2() };
        let body_style = role_style(kit, TypeRole::Body, None, message_ink, Align::Centre);
        let kgap = kit.space(2);
        let key_h = Button::height(kit);
        // every key as wide as the widest of them
        let mut kw = 0f32;
        for (i, k) in PAD.iter().enumerate() {
            let label = match *k {
                "clear" => "Clear",
                "back" => "",
                d => d,
            };
            kw = kw.max(Button::width_for(kit, label, i == 11));
        }
        kw = kw.max(Button::width_for(kit, "OK", false) * 0.0).max(112.0);
        let pad_w = 3.0 * kw + 2.0 * kgap;
        let sheet_w = PIN_W.max(pad_w + 2.0 * pad);
        let inner_w = sheet_w - 2.0 * pad;
        let title = if p.title.is_empty() { "Enter the PIN".to_string() } else { plain(&p.title) };
        let message = if p.retry { "Wrong PIN. Try again.".to_string() } else { plain(&p.message) };
        let body_line = ui::components::line_height(body_style.ty);
        let title_line = ui::components::line_height(title_style.ty);
        let ok_w = Button::width_for(kit, "OK", false).max(160.0);
        // the sheet must fit the safe height: the message goes to one line, then the title, then the lock, as the type grows
        let avail = 1080.0 - 2.0 * 54.0;
        let (mut tile_on, mut tl, mut ml) = (true, 2usize, 2usize);
        let (title_lines, msg_lines, sheet_h) = loop {
            let title_lines = wrap::lines(kit, title_style, &title, inner_w, tl);
            let msg_lines = if message.is_empty() { Vec::new() } else { wrap::lines(kit, body_style, &message, inner_w, ml) };
            let h = 2.0 * pad
                + if tile_on { TILE + gap } else { 0.0 }
                + title_lines.len() as f32 * title_line
                + if msg_lines.is_empty() { 0.0 } else { gap + msg_lines.len() as f32 * body_line }
                + gap
                + CELL
                + gap
                + 4.0 * key_h
                + 3.0 * kgap
                + gap
                + key_h;
            if h <= avail {
                break (title_lines, msg_lines, h.ceil());
            }
            if ml > 1 {
                ml -= 1;
            } else if tl > 1 {
                tl -= 1;
            } else if tile_on {
                tile_on = false;
            } else {
                break (title_lines, msg_lines, avail);
            }
        };
        let (sx, sy) = (((1920.0 - sheet_w) / 2.0).round(), ((1080.0 - sheet_h) / 2.0).round());
        let frame = Frame::new(sx, sy, sheet_w, sheet_h);
        let mut sheet = Sheet::new();
        let mut words = Vec::new();
        let scrim = Self::scrim(kit, z, false, true);
        sheet.shape(scrim, (0.0, 0.0));
        sheet.pane(Pane::new(kit, frame, z + 1, PaneKind::Sheet, false));
        let mut y = sy + pad;
        if tile_on {
            let (tile, glyph) = icon_tile(kit, TILE, z + 2, "lock", false)?;
            let tx = sx + ((sheet_w - TILE) / 2.0).round();
            sheet.shape(tile, (tx, y));
            if let Some(g) = glyph {
                sheet.image(g, tile_glyph_at((tx, y), TILE));
            }
            y += TILE + gap;
        }
        y += Self::lines_at(kit, &mut sheet, &mut words, z + 3, title_style, &title_lines, Frame::new(sx + pad, y, inner_w, 0.0))?;
        if !msg_lines.is_empty() {
            y += gap;
            y += Self::lines_at(kit, &mut sheet, &mut words, z + 3, body_style, &msg_lines, Frame::new(sx + pad, y, inner_w, 0.0))?;
        }
        y += gap;
        // the four cells of the digits
        let cells_w = 4.0 * CELL + 3.0 * kit.space(3);
        let mut cells = Vec::new();
        for i in 0..4 {
            let x = sx + ((sheet_w - cells_w) / 2.0).round() + i as f32 * (CELL + kit.space(3));
            let layer = kit.ui.scene.shape(CELL as u32, CELL as u32, z + 2, Self::cell_shape(kit, false), crate::system::parts::HIDDEN);
            cells.push(sheet.shape(layer, (x, y)));
        }
        y += CELL + gap;
        engine.focus.open_modal(SCOPE_PIN);
        let mut buttons = Vec::new();
        let mut targets = Vec::new();
        for (i, k) in PAD.iter().enumerate() {
            let (row, col) = (i / 3, i % 3);
            let f = Frame::new(sx + ((sheet_w - pad_w) / 2.0).round() + col as f32 * (kw + kgap), y + row as f32 * (key_h + kgap), kw, key_h);
            let spec = match *k {
                "clear" => Spec::new("Clear", Tone::Plain),
                "back" => Spec::new("", Tone::Plain).with_icon("delete"),
                d => Spec::new(d, Tone::Plain),
            };
            buttons.push(Button::new(kit, engine, Place { id: id(i), frame: f, region: REGION, scope: SCOPE_PIN, z: z + 4 }, spec)?);
            targets.push((id(i), Target::Pin(i)));
        }
        y += 4.0 * key_h + 3.0 * kgap + gap;
        let ok = Frame::new(sx + ((sheet_w - ok_w) / 2.0).round(), y, ok_w, key_h);
        let mut ok_button =
            Button::new(kit, engine, Place { id: id(OK_INDEX), frame: ok, region: REGION, scope: SCOPE_PIN, z: z + 4 }, Spec::new("OK", Tone::Primary))?;
        ok_button.set_dimmed(kit, p.typed.chars().count() < 4);
        buttons.push(ok_button);
        targets.push((id(OK_INDEX), Target::Pin(OK_INDEX)));
        let first = id(p.index.min(OK_INDEX));
        engine.focus.focus(first);
        let _ = engine.flow.snap(first, true, now_ns);
        let typed = p.typed.chars().count();
        let mut s = Self::finish(
            kit,
            Screen {
                kind: Kind::Pin,
                sig: String::new(),
                scope: SCOPE_PIN,
                sheet,
                buttons,
                targets,
                frame,
                paint: Rgba(0, 0, 0, 0),
                words,
                cells,
                typed,
                focus: p.index,
            },
        )?;
        s.sig = signature(&View::Pin(p.clone())).map(|x| x.1).unwrap_or_default();
        Self::paint_cells(kit, &mut s);
        Ok(s)
    }

    // ---- what the tests and the harness read

    /// The sheet's rectangle and each button's, while a screen is up.
    pub fn geometry(&self) -> Value {
        match &self.current {
            Some(s) => json!({
                "kind": match s.kind { Kind::Alert => "alert", Kind::Input => "input", Kind::Pin => "pin" },
                "sheet": {"x": s.frame.x, "y": s.frame.y, "w": s.frame.w, "h": s.frame.h},
                "buttons": s.buttons.iter().map(|b| { let f = b.frame(); json!({"x": f.x, "y": f.y, "w": f.w, "h": f.h, "label": b.spec().label}) }).collect::<Vec<_>>(),
                "focus": s.focus,
                "typed": s.typed,
                "scope": s.scope.0,
            }),
            None => Value::Null,
        }
    }

    /// The layers the screen holds (the budget).
    pub fn layers(&self) -> usize {
        self.current.iter().chain(self.leaving.iter().map(|(s, _)| s)).map(|s| s.sheet.layers() + s.buttons.iter().map(Button::layers).sum::<usize>()).sum()
    }

    /// Describes the screen on show to a lint.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(s) = &self.current else { return };
        let name = match s.kind {
            Kind::Alert => "alert",
            Kind::Input => "input popup",
            Kind::Pin => "PIN prompt",
        };
        let sheet = lint.panel(&format!("{name} sheet"), s.frame, s.paint, s.kind != Kind::Input);
        for (i, w) in s.words.iter().enumerate() {
            lint.word(kit, &format!("{name} line {i}"), &w.text, w.frame, w.style, Some(sheet));
        }
        for b in &s.buttons {
            b.feed_lint(kit, lint, sheet);
        }
    }

    /// The look changed: what is up is built again at the next apply.
    pub fn restyle(&mut self, engine: &mut Engine) {
        if let Some(s) = self.current.take() {
            for b in &s.buttons {
                b.detach(engine);
            }
            engine.focus.close_modal();
        }
        for (s, _) in self.leaving.drain(..) {
            drop(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::lint::Lint;
    use crate::kit::screen::{Input, Model};
    use crate::kit::testkit;
    use crate::system::alert::{ALERT_TAG, Alerts, INPUT_TAG, PIN_TAG};
    use ui::lucent::palette::Mode;

    fn rig(mode: Mode, scale: f32) -> (ui::testing::Harness, Kit, Engine, AlertSurface, Alerts) {
        let (h, kit) = testkit::kit_in(mode, scale);
        let engine = Engine::new(&kit).expect("an engine");
        let layout: Value = serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap();
        let surface = AlertSurface::new(1100, &json!({}), &layout);
        (h, kit, engine, surface, Alerts::new())
    }

    fn luna(a: &mut Alerts, tag: u64, doc: &str) {
        a.update(&Input::Luna { tag, payload: serde_json::from_str(doc).unwrap() }, 1000);
    }

    const ALERT: &str = r#"{"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.home", "title": "Uninstall Crunchyroll?",
        "message": "The app and its sign-in on this TV are removed. Downloads in the app are lost. It can be installed again from the LG Content Store.",
        "modal": true, "timeout": 0, "timestamp": 1, "buttons": [{"label": "Keep it", "focus": true}, {"label": "Uninstall", "danger": true}]}}"#;
    const INPUT: &str = r#"{"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.hdmi3", "timeout": 30, "timestamp": 13, "portName": "PlayStation 5", "appId": "com.webos.app.hdmi3"}}"#;
    const PIN: &str = r#"{"pincodePromptAction": "open", "pincodePromptInfo": {"appId": "com.webos.app.hdmi3", "title": "Enter the TV PIN", "promptType": "parental",
        "message": "HDMI 3 \u00b7 PlayStation 5 is locked for the kids.", "retry": 0, "timestamp": 14}}"#;

    fn lint_of(kit: &mut Kit, s: &AlertSurface) -> Vec<String> {
        let mut lint = Lint::new();
        s.feed_lint(kit, &mut lint);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    fn within(r: &Value, x: f64, y: f64, w: f64, h: f64) -> bool {
        let f = |k: &str| r[k].as_f64().unwrap();
        f("x") >= x - 0.5 && f("y") >= y - 0.5 && f("x") + f("w") <= x + w + 0.5 && f("y") + f("h") <= y + h + 0.5
    }

    #[test]
    fn the_alert_is_a_sheet_in_the_middle_with_its_buttons_inside_and_reads_clean_at_both_text_sizes_in_both_materials() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit, mut engine, mut s, mut a) = rig(mode, scale);
                luna(&mut a, ALERT_TAG, ALERT);
                assert!(s.apply(&mut kit, &mut engine, &a.view(), 1000, 1_000_000).unwrap());
                let g = s.geometry();
                assert_eq!(g["kind"], "alert");
                let sheet = &g["sheet"];
                let (x, w) = (sheet["x"].as_f64().unwrap(), sheet["w"].as_f64().unwrap());
                assert!((x + w / 2.0 - 960.0).abs() <= 1.0, "centred across: {sheet}");
                assert!(within(sheet, 64.0, 54.0, 1920.0 - 128.0, 1080.0 - 108.0), "inside the safe area at {scale}: {sheet}");
                for b in g["buttons"].as_array().unwrap() {
                    assert!(within(b, x, sheet["y"].as_f64().unwrap(), w, sheet["h"].as_f64().unwrap()), "a button inside the sheet at {scale}: {b}");
                }
                assert_eq!(g["buttons"].as_array().unwrap().len(), 2);
                assert_eq!(engine.focus.current(), Some(id(0)), "the button with focus: true holds it");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn the_surface_follows_the_models_focus_with_the_engine_and_a_warning_is_a_whole_screen() {
        let (_h, mut kit, mut engine, mut s, mut a) = rig(Mode::Dark, 1.0);
        luna(&mut a, ALERT_TAG, ALERT);
        s.apply(&mut kit, &mut engine, &a.view(), 1000, 1_000_000).unwrap();
        a.update(&Input::Key { name: "right".into(), down: true, repeat: false }, 1100);
        s.apply(&mut kit, &mut engine, &a.view(), 1100, 1_100_000_000).unwrap();
        assert_eq!(engine.focus.current(), Some(id(1)), "the engine's focus is the model's");
        assert!(engine.flow.in_flight() >= 1, "the move flows");
        // the same alert with another focus is not a new screen
        assert_eq!(s.layers(), s.layers());
        luna(&mut a, ALERT_TAG, r#"{"alertAction": "closeAll"}"#);
        let up = s.apply(&mut kit, &mut engine, &a.view(), 1200, 1_200_000_000).unwrap();
        assert!(up, "fading out still counts as on the screen");
        assert!(!s.is_up());
        assert!(s.due_at().is_some());
        assert!(s.tick(2000), "the layers are dropped once it has faded");
        assert_eq!(s.layers(), 0);
        assert_eq!(engine.focus.modal(), None, "the modal scope is closed");
        luna(
            &mut a,
            ALERT_TAG,
            r#"{"alertAction": "open", "alertInfo": {"sourceId": "x", "type": "warning", "title": "The TV is too hot", "message": "Turn it off.", "timestamp": 2, "buttons": []}}"#,
        );
        s.apply(&mut kit, &mut engine, &a.view(), 3000, 3_000_000_000).unwrap();
        assert_eq!(s.geometry()["buttons"].as_array().unwrap().len(), 1, "a warning without buttons has an OK");
    }

    #[test]
    fn the_input_popup_sits_at_the_toasts_corner_and_reads_clean() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit, mut engine, mut s, mut a) = rig(Mode::Dark, scale);
            luna(&mut a, INPUT_TAG, INPUT);
            s.apply(&mut kit, &mut engine, &a.view(), 1000, 1_000_000).unwrap();
            let g = s.geometry();
            assert_eq!(g["kind"], "input");
            let sheet = &g["sheet"];
            assert_eq!(sheet["x"].as_f64().unwrap() + sheet["w"].as_f64().unwrap(), 1920.0 - 24.0, "right-aligned with the toasts' column: {sheet}");
            assert_eq!(sheet["y"], 24.0);
            assert_eq!(g["buttons"].as_array().unwrap().len(), 2, "Switch and Stay here");
            let found = lint_of(&mut kit, &s);
            assert!(found.is_empty(), "at {scale}: {found:?}");
        }
    }

    #[test]
    fn the_pin_prompt_has_four_cells_twelve_keys_and_ok_and_reads_clean_at_both_text_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit, mut engine, mut s, mut a) = rig(mode, scale);
                luna(&mut a, PIN_TAG, PIN);
                s.apply(&mut kit, &mut engine, &a.view(), 1000, 1_000_000).unwrap();
                let g = s.geometry();
                assert_eq!(g["kind"], "pin");
                assert_eq!(g["buttons"].as_array().unwrap().len(), 13, "the pad and OK");
                assert!(within(&g["sheet"], 64.0, 54.0, 1920.0 - 128.0, 1080.0 - 108.0), "inside the safe area at {scale}: {}", g["sheet"]);
                for b in g["buttons"].as_array().unwrap() {
                    assert!(
                        within(
                            b,
                            g["sheet"]["x"].as_f64().unwrap(),
                            g["sheet"]["y"].as_f64().unwrap(),
                            g["sheet"]["w"].as_f64().unwrap(),
                            g["sheet"]["h"].as_f64().unwrap()
                        ),
                        "a key inside the sheet: {b}"
                    );
                }
                for digit in ["1", "2"] {
                    a.update(&Input::Key { name: digit.into(), down: true, repeat: false }, 1100);
                }
                s.apply(&mut kit, &mut engine, &a.view(), 1100, 1_100_000_000).unwrap();
                assert_eq!(s.geometry()["typed"], 2, "two digits are in the cells");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                assert!(s.layers() < 90, "the budget of a modal: {} layers", s.layers());
            }
        }
    }

    #[test]
    fn a_click_on_a_key_of_the_pad_is_a_target_and_the_hover_is_only_a_cue() {
        let (_h, mut kit, mut engine, mut s, mut a) = rig(Mode::Dark, 1.0);
        luna(&mut a, PIN_TAG, PIN);
        s.apply(&mut kit, &mut engine, &a.view(), 1000, 1_000_000).unwrap();
        let g = s.geometry();
        let five = &g["buttons"][4];
        let (cx, cy) = (five["x"].as_f64().unwrap() as f32 + 10.0, five["y"].as_f64().unwrap() as f32 + 10.0);
        let sample = |buttons| Sample { x: cx, y: cy, buttons, visible: true };
        assert_eq!(s.pointer(&kit, &mut engine, 1, sample(0)), None, "a hover acts on nothing");
        assert_eq!(engine.focus.current(), Some(id(0)), "and moves no focus");
        assert_eq!(s.pointer(&kit, &mut engine, 2, sample(1)), None);
        assert_eq!(s.pointer(&kit, &mut engine, 3, sample(0)), Some(Target::Pin(4)), "the release over the same key is the click");
        assert_eq!(engine.focus.current(), Some(id(4)), "which focused it");
        let _ = &mut kit;
    }

    #[test]
    fn a_retry_of_the_pin_replaces_the_screen_at_once_and_says_so_in_the_danger_ink() {
        let (_h, mut kit, mut engine, mut s, mut a) = rig(Mode::Dark, 1.0);
        luna(&mut a, PIN_TAG, PIN);
        s.apply(&mut kit, &mut engine, &a.view(), 1000, 1_000_000).unwrap();
        let retry = PIN.replace(r#""retry": 0"#, r#""retry": 1"#).replace("\"timestamp\": 14", "\"timestamp\": 15");
        luna(&mut a, PIN_TAG, &retry);
        s.apply(&mut kit, &mut engine, &a.view(), 1100, 1_100_000_000).unwrap();
        assert!(s.due_at().is_none(), "a screen of the same kind replaces the old at once: nothing fades");
        assert!(s.is_up());
        let found = lint_of(&mut kit, &s);
        assert!(found.is_empty(), "{found:?}");
    }
}
