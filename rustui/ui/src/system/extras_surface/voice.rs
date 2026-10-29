//! The push-to-talk voice bar drawn (`panels/VoiceBar.qml`) and the mark that says the microphone is open:
//!
//! - the **bar**: a glass card 760 wide at the foot, 64 from the left, with the microphone on a round tile (the accent while it listens, a wash
//!   after), the state in a caption ("Listening", "Working it out", "Voice search" for a failure) and the words under it ("Speak now\u{2026}",
//!   "One moment\u{2026}", or why it did not work, in the helper's own words). While it listens the key to hold is a glyph at the right (the voice
//!   key, held), not a sentence; while it works a ring of dots turns round the tile.
//! - the **mark**: a round glass tile at the bottom right with the microphone in the positive colour and a dot, for as long as the microphone is
//!   open (the TV never listens without it).
//!
//! The bar is made once for a look and a text size; a change of state changes its words, its tile and its ring in place, so it does not fade in
//! again when "Listening" becomes "Working it out".

use super::spin_head;
use super::spinner::Spinner;
use crate::kit::button::faded;
use crate::kit::hint::{Hint, HintRow, Key, RowAt};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style};
use crate::kit::widgets::{edge, selected_wash};
use crate::kit::{Kit, wrap};
use crate::panels::ui::{Arrive, Root, WordLine, Words};
use crate::system::extras::{VoiceState, VoiceView};
use compositor_protocol::{Fill, Props, Shape};
use lumatv_tokens::TypeRole;
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const FADE_MS: u32 = 140;
const BAR_W: f32 = 760.0;
const MIC: f32 = 72.0;
const RING: f32 = 88.0;
const MARK: f32 = 72.0;
const MIC_ICON: u32 = 34;

fn caption_of(state: &VoiceState) -> &'static str {
    match state {
        VoiceState::Listening => "Listening",
        VoiceState::Working => "Working it out",
        _ => "Voice search",
    }
}

fn words_of(state: &VoiceState) -> String {
    match state {
        VoiceState::Listening => "Speak now\u{2026}".into(),
        VoiceState::Working => "One moment\u{2026}".into(),
        VoiceState::Failed(w) => w.clone(),
        VoiceState::Idle => String::new(),
    }
}

/// What the held key looks like: the voice key, held.
fn key_hint() -> Hint {
    Hint { keys: vec![(Key::Mic, true)], action: None, label: None }
}

#[derive(Clone, Debug, PartialEq)]
struct Lay {
    bar: Frame,
    tile: Frame,
    ring: Frame,
    caption: Frame,
    title_at: (f32, f32, f32),
    title_lines: usize,
    hint_at: (f32, f32, f32),
    hint_w: f32,
}

fn layout(kit: &mut Kit, words: &str) -> Lay {
    let (s1, s2, s4) = (kit.space(1), kit.space(2), kit.space(4));
    let ink = kit.ui.look.ink();
    let title_style = role_style(kit, TypeRole::Body, None, ink, Align::Left);
    let (caption_h, title_h) = (kit.line(TypeRole::Caption), kit.line(TypeRole::Body));
    let hint_w = crate::panels::ui::hint_width(kit, &key_hint(), TypeRole::Label);
    let text_x_off = 18.0 + MIC + 20.0;
    let gap = s4;
    // the bar is 760 wide, wider when the text size asks for it; the words may take two lines when they are a failure's
    let mut w = BAR_W;
    let mut lines = wrap::lines(kit, title_style, words, w - text_x_off - 26.0 - hint_w - gap, 2);
    if lines.len() > 1 {
        w = 1100.0;
        lines = wrap::lines(kit, title_style, words, w - text_x_off - 26.0, 2);
    }
    let n = lines.len().max(1);
    let h = 110.0f32.max(2.0 * s4 + s1 + caption_h + n as f32 * title_h).max(RING + 2.0 * s2).ceil();
    let bar = Frame::new(64.0, 1080.0 - 56.0 - h, w, h);
    let cy = bar.y + h / 2.0;
    let tile = Frame::new(bar.x + 18.0, (cy - MIC / 2.0).round(), MIC, MIC);
    let ring = Frame::new(bar.x + 10.0, (cy - RING / 2.0).round(), RING, RING);
    let block_h = s1 + caption_h + n as f32 * title_h;
    let top = (cy - block_h / 2.0).round();
    let text_w = w - text_x_off - 26.0 - hint_w - gap;
    let caption = Frame::new(bar.x + text_x_off, top, text_w.max(80.0), caption_h);
    let title_at = (bar.x + text_x_off, top + s1 + caption_h, if n > 1 { w - text_x_off - 26.0 } else { text_w.max(80.0) });
    let hint_at = (bar.right() - 26.0 - hint_w, bar.y, h);
    Lay { bar, tile, ring, caption, title_at, title_lines: n, hint_at, hint_w }
}

struct BarBuilt {
    sig: String,
    lay: Lay,
    _pane: Pane,
    ring: ShapeLayer,
    spin: Spinner,
    tile: ShapeLayer,
    mic: Option<ImageLayer>,
    caption: Text,
    title: Words,
    hint: HintRow,
    state: Option<VoiceState>,
}

struct MarkBuilt {
    sig: String,
    frame: Frame,
    _pane: Pane,
    _mic: Option<ImageLayer>,
    _dot: ShapeLayer,
}

pub struct VoiceSurface {
    z: i32,
    bar_root: Option<Root>,
    bar: Option<BarBuilt>,
    bar_leaving: Option<u64>,
    mark_root: Option<Root>,
    mark: Option<MarkBuilt>,
    mark_leaving: Option<u64>,
}

impl VoiceSurface {
    pub fn new(z: i32) -> Self {
        Self { z, bar_root: None, bar: None, bar_leaving: None, mark_root: None, mark: None, mark_leaving: None }
    }

    pub fn is_up(&self) -> bool {
        self.bar_root.is_some() || self.mark_root.is_some()
    }

    fn tile_shape(kit: &Kit, listening: bool) -> Shape {
        if listening {
            plate::solid(kit.ui.look.accent(), MIC / 2.0)
        } else {
            Shape { corner_radius: MIC / 2.0, fill: Fill::Solid(selected_wash(kit)), border_width: 1.0, border: edge(kit) }
        }
    }

    fn build_bar(kit: &mut Kit, root: &Root, z: i32, lay: Lay, words: &str, sig: String) -> std::io::Result<BarBuilt> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        let caption_style = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left);
        let title_style = role_style(kit, TypeRole::Body, None, ink, Align::Left);
        let lines = wrap::lines(kit, title_style, words, lay.title_at.2, lay.title_lines);
        scene.adopting_view(&root.group, || -> std::io::Result<BarBuilt> {
            let pane = Pane::new(kit, lay.bar, z, PaneKind::Card, false);
            let ring = plate::shape(&kit.ui, lay.ring, z + 1, plate::ring(faded(accent, 0.45), RING / 2.0, 3.0));
            let spin = Spinner::new(kit, lay.ring.x, lay.ring.y, RING, 5.0, z + 1, accent);
            let tile = plate::shape(&kit.ui, lay.tile, z + 2, Self::tile_shape(kit, false));
            let mic = icon::layer(kit, "mic", MIC_ICON, ink, z + 3, Props::at((lay.tile.x + (MIC - MIC_ICON as f32) / 2.0).round(), (lay.tile.y + (MIC - MIC_ICON as f32) / 2.0).round()))?;
            let caption = Text::new(kit, lay.caption, z + 2, caption_style, "Voice search")?;
            let title_lines: Vec<WordLine> = (0..lay.title_lines).map(|i| WordLine::new(lines.get(i).map_or("", String::as_str), TypeRole::Body, ink)).collect();
            let title = Words::new(kit, lay.title_at.0, lay.title_at.1, lay.title_at.2, z + 2, &title_lines)?;
            let hint = HintRow::new(kit, &key_hint(), TypeRole::Label, RowAt { x: lay.hint_at.0, y: lay.hint_at.1, h: lay.hint_at.2, z: z + 2 }, kit.ui.look.ink3())?;
            Ok(BarBuilt { sig, lay: lay.clone(), _pane: pane, ring, spin, tile, mic, caption, title, hint, state: None })
        })
    }

    /// The state of the bar changes its words, its tile and its ring where they are.
    fn show_state(kit: &mut Kit, b: &mut BarBuilt, state: &VoiceState, now_ms: u64) {
        let listening = *state == VoiceState::Listening;
        let working = *state == VoiceState::Working;
        if b.state.as_ref() != Some(state) {
            b.state = Some(state.clone());
            b.caption.set(kit, caption_of(state));
            let title_style = role_style(kit, TypeRole::Body, None, kit.ui.look.ink(), Align::Left);
            let lines = wrap::lines(kit, title_style, &words_of(state), b.lay.title_at.2, b.lay.title_lines);
            for i in 0..b.title.len() {
                b.title.set(kit, i, lines.get(i).map_or("", String::as_str));
            }
            b.tile.reshape(Self::tile_shape(kit, listening));
            if let Some(m) = b.mic.as_mut() {
                let ink = if listening { kit.ui.look.accent_ink() } else { kit.ui.look.ink() };
                icon::repaint(kit, m, "mic", ink);
            }
            b.hint.set_visible(listening);
            b.ring.set(if working { crate::panels::ui::HIDDEN } else { Props::at(b.lay.ring.x, b.lay.ring.y) });
            b.spin.show(working);
        }
        if working && !kit.ui.look.reduced_motion {
            b.spin.step(spin_head(now_ms));
        }
    }

    fn build_mark(kit: &mut Kit, root: &Root, z: i32, sig: String) -> std::io::Result<MarkBuilt> {
        let scene = kit.ui.scene.clone();
        let positive = kit.ui.look.positive();
        let frame = Frame::new(1920.0 - 64.0 - MARK, 1080.0 - 56.0 - MARK, MARK, MARK);
        scene.adopting_view(&root.group, || -> std::io::Result<MarkBuilt> {
            let pane = Pane::new(kit, frame, z, PaneKind::Pill, false);
            let mic = icon::layer(kit, "mic", 32, positive, z + 1, Props::at((frame.x + (MARK - 32.0) / 2.0).round(), (frame.y + (MARK - 32.0) / 2.0).round()))?;
            let dot = plate::shape(&kit.ui, Frame::new(frame.right() - 22.0, frame.y + 8.0, 12.0, 12.0), z + 2, plate::solid(positive, 6.0));
            Ok(MarkBuilt { sig, frame, _pane: pane, _mic: mic, _dot: dot })
        })
    }

    /// Fits the bar and the mark to the view. Returns whether anything of them is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, bar: Option<&VoiceView>, mic: bool, now_ms: u64) -> std::io::Result<bool> {
        let fade = kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS);
        match bar {
            Some(v) => {
                self.bar_leaving = None;
                let words = words_of(&v.state);
                let lay = layout(kit, if matches!(v.state, VoiceState::Failed(_)) { &words } else { "" });
                let sig = format!("{}|{}|{:?}", kit.generation(), kit.text_scale, lay);
                if self.bar.as_ref().is_some_and(|b| b.sig != sig) {
                    self.bar = None;
                    self.bar_root = None;
                }
                if self.bar_root.is_none() {
                    self.bar_root = Some(Root::new(kit, Arrive::Fade, false)?);
                }
                if self.bar.is_none() {
                    let root = self.bar_root.as_ref().expect("a root");
                    let built = Self::build_bar(kit, root, self.z, lay, &words, sig)?;
                    if kit.ui.scene.take_overflow() {
                        return Err(std::io::Error::other("the scene is full: the voice bar could not be drawn whole"));
                    }
                    self.bar = Some(built);
                }
                if let Some(root) = self.bar_root.as_mut().filter(|r| !r.is_shown()) {
                    root.show(fade);
                }
                if let Some(b) = self.bar.as_mut() {
                    Self::show_state(kit, b, &v.state, now_ms);
                }
            }
            None => Self::leave(&mut self.bar_root, &mut self.bar_leaving, fade, now_ms, &mut self.bar),
        }
        if mic {
            self.mark_leaving = None;
            let sig = format!("{}|{}", kit.generation(), kit.text_scale);
            if self.mark.as_ref().is_some_and(|m| m.sig != sig) {
                self.mark = None;
                self.mark_root = None;
            }
            if self.mark_root.is_none() {
                self.mark_root = Some(Root::new(kit, Arrive::Fade, false)?);
            }
            if self.mark.is_none() {
                let root = self.mark_root.as_ref().expect("a root");
                self.mark = Some(Self::build_mark(kit, root, self.z + 10, sig)?);
            }
            if let Some(root) = self.mark_root.as_mut().filter(|r| !r.is_shown()) {
                root.show(fade);
            }
        } else {
            Self::leave(&mut self.mark_root, &mut self.mark_leaving, fade, now_ms, &mut self.mark);
        }
        Ok(self.is_up())
    }

    fn leave<B>(root: &mut Option<Root>, leaving: &mut Option<u64>, fade: u32, now_ms: u64, built: &mut Option<B>) {
        if let Some(r) = root.as_mut()
            && leaving.is_none()
        {
            r.hide(fade);
            *leaving = Some(now_ms + u64::from(fade) + 60);
        }
        if leaving.is_some_and(|t| now_ms >= t) {
            *built = None;
            *root = None;
            *leaving = None;
        }
    }

    /// The ring turns in place while the bar works; what has faded out is dropped. Returns whether a redraw is wanted.
    pub fn tick(&mut self, kit: &Kit, now_ms: u64) -> bool {
        let mut redraw = false;
        if let Some(b) = self.bar.as_mut().filter(|b| b.state == Some(VoiceState::Working) && !kit.ui.look.reduced_motion) {
            b.spin.step(spin_head(now_ms));
        }
        if self.bar_leaving.is_some_and(|t| now_ms >= t) {
            Self::leave(&mut self.bar_root, &mut self.bar_leaving, 0, now_ms, &mut self.bar);
            redraw = true;
        }
        if self.mark_leaving.is_some_and(|t| now_ms >= t) {
            Self::leave(&mut self.mark_root, &mut self.mark_leaving, 0, now_ms, &mut self.mark);
            redraw = true;
        }
        redraw
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.bar_leaving, self.mark_leaving].into_iter().flatten().min()
    }

    pub fn restyle(&mut self) {
        self.bar = None;
        self.bar_root = None;
        self.bar_leaving = None;
        self.mark = None;
        self.mark_root = None;
        self.mark_leaving = None;
    }

    pub fn geometry(&self) -> Value {
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        json!({
            "bar": self.bar.as_ref().map(|b| json!({"frame": f(b.lay.bar), "tile": f(b.lay.tile), "caption": b.caption.text(), "title": (0..b.title.len()).map(|i| b.title.text(i).to_string()).collect::<Vec<_>>(),
                                                     "listening": b.state == Some(VoiceState::Listening), "working": b.state == Some(VoiceState::Working)})),
            "mark": self.mark.as_ref().map(|m| f(m.frame)),
        })
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        if let Some(b) = &self.bar {
            let panel = lint.panel("voice bar", b.lay.bar, kit.ui.look.colour(PaletteRole::Sheet), true);
            let caption = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), kit.ui.look.ink2(), Align::Left);
            lint.word(kit, "voice caption", b.caption.text(), b.caption.frame(), caption, Some(panel));
            b.title.feed_lint(kit, lint, "voice words", Some(panel), (0.0, 0.0));
        }
        if let Some(m) = &self.mark {
            lint.panel("microphone mark", m.frame, kit.ui.look.colour(PaletteRole::Sheet), true);
        }
    }
}
