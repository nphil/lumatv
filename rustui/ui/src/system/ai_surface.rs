//! The AI access prompt drawn (`AiPrompt.qml`, `docs/mocks/05-ai-request-prompt`): a sheet in the middle over a dim, with who is asking, the title
//! in the helper's words, what will happen, the assistant's own reason apart and labelled as its, the two keys as glyph hints (OK, Back: what each
//! would do, never the words "OK" and "Back"), and how long is left. Nothing on it is a control: it has no button, takes no focus and answers
//! nothing ([`super::ai`], Principle 0); the hints say what the physical remote's keys do, which the helper reads from the input device itself.
//!
//! The sheet is as wide as its words need so the helper's own words are whole: 880 px, then wider as the text size grows, never cut in the title
//! or in what will happen; the assistant's reason, which is not the helper's, is the one that gives way if no width holds the card in the
//! safe area. It is built when a prompt comes (or its words, look or text size change) and dropped a moment after it has gone.

use super::ai::{Prompt, View};
use crate::kit::Kit;
use crate::kit::button::faded;
use crate::kit::hint::{Hint, HintRow, Key, RowAt, glyph_side};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::edge;
use crate::kit::wrap;
use crate::panels::ui::{Arrive, Root, WordLine, Words};
use compositor_protocol::{Fill, Props, Rect, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

/// The widths the sheet may take, narrowest first.
const WIDTHS: [f32; 3] = [880.0, 1100.0, 1320.0];
/// The tallest the sheet may be: the screen less Home's safe area above and below.
const MAX_H: f32 = 1080.0 - 2.0 * 54.0;
const FADE_MS: u32 = 140;

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    card: Frame,
    from_icon: (f32, f32, u32),
    from: (f32, f32, f32),
    from_text: String,
    title: Vec<String>,
    title_at: (f32, f32, f32),
    what: Frame,
    what_icon: (f32, f32, u32),
    what_label: (f32, f32, f32),
    detail: Vec<String>,
    detail_at: (f32, f32, f32),
    reason_label: Option<(f32, f32, f32)>,
    reason: Vec<String>,
    reason_at: (f32, f32, f32),
    hints_at: (f32, f32, f32),
    clock_icon: (f32, f32, u32),
    secs_at: Frame,
    bar: Frame,
}

/// How many lines each of the words may take before they are cut.
#[derive(Clone, Copy)]
struct Caps {
    title: usize,
    detail: usize,
    reason: usize,
}

fn hints() -> [Hint; 2] {
    [Hint::labelled(Key::Ok, "Allow once"), Hint::labelled(Key::Back, "Deny")]
}

fn plan(kit: &mut Kit, p: &Prompt, width: f32, caps: Caps) -> Layout {
    let (s2, s3, s4, s5, s7) = (kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(7));
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let inner = width - 2.0 * s7;
    let caption = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left);
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let body_style = role_style(kit, TypeRole::Body, None, ink, Align::Left);
    let reason_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
    let (cap_h, title_lh, body_lh) = (kit.line(TypeRole::Caption), kit.line(TypeRole::Title), kit.line(TypeRole::Body));
    let text_scale = kit.text_scale;
    let scale_icon = move |base: f32| (base * (1.0 + 0.6 * (text_scale - 1.0))).round();
    let (x, mut y) = (0.0f32, s7);
    // who asks, and on the right how long is left: a clock, the seconds, and a bar that empties
    let from_side = scale_icon(26.0);
    let from_h = from_side.max(cap_h);
    let secs_w = width_of(kit, caption, "100 s");
    let bar_w = scale_icon(96.0);
    let bar_h = kit.space(1).max(4.0);
    let group_w = from_side + s2 + secs_w + s3 + bar_w;
    let bar = Frame::new(x + s7 + inner - bar_w, y + ((from_h - bar_h) / 2.0).round(), bar_w, bar_h);
    let secs_at = Frame::new(bar.x - s3 - secs_w, y + ((from_h - cap_h) / 2.0).round(), secs_w, cap_h);
    let clock_icon = (secs_at.x - s2 - from_side, y + ((from_h - from_side) / 2.0).round(), from_side as u32);
    let from_icon = (x + s7, y + ((from_h - from_side) / 2.0).round(), from_side as u32);
    let from_w = inner - from_side - s3 - group_w - s5;
    let full = p.from_line();
    let from_text = if width_of(kit, caption, &full) <= from_w {
        full
    } else {
        let short = p.from_short();
        if width_of(kit, caption, &short) <= from_w { short } else { wrap::elide(kit, caption, &short, from_w) }
    };
    let from = (x + s7 + from_side + s3, y + ((from_h - cap_h) / 2.0).round(), from_w);
    y += from_h + s4;
    // the title
    let title = wrap::lines(kit, title_style, &p.title, inner, caps.title);
    let title_at = (x + s7, y, inner);
    y += title.len() as f32 * title_lh + s4;
    // what will happen
    let what_pad = s5;
    let icon_side = scale_icon(32.0);
    let text_x = x + s7 + what_pad + icon_side + s4;
    let text_w = inner - what_pad - icon_side - s4 - what_pad;
    let detail = wrap::lines(kit, body_style, &p.detail, text_w, caps.detail);
    let what_h = (what_pad + cap_h + detail.len() as f32 * body_lh + what_pad).max(icon_side + 2.0 * what_pad).ceil();
    let what = Frame::new(x + s7, y, inner, what_h);
    let what_icon = (x + s7 + what_pad, y + what_pad, icon_side as u32);
    let what_label = (text_x, y + what_pad, text_w);
    let detail_at = (text_x, y + what_pad + cap_h, text_w);
    y += what_h + s4;
    // the assistant's reason, labelled as its own
    let (reason_label, reason, reason_at) = if p.reason.is_empty() {
        (None, Vec::new(), (0.0, 0.0, 0.0))
    } else {
        let label = (x + s7, y, inner);
        let quoted = format!("\u{201c}{}\u{201d}", p.reason);
        let lines = wrap::lines(kit, reason_style, &quoted, inner, caps.reason);
        let at = (x + s7, y + cap_h, inner);
        y += cap_h + lines.len() as f32 * body_lh + s4;
        (Some(label), lines, at)
    };
    // what the two keys do
    let hint_h = glyph_side(kit, TypeRole::Label).max(kit.line(TypeRole::Label) as u32) as f32;
    let hints_at = (x + s7, y + s2, hint_h);
    y += hint_h + s2 + s7;
    let h = y.ceil();
    let (cx, cy) = (((1920.0 - width) / 2.0).round(), ((1080.0 - h) / 2.0).round());
    let at = |f: Frame| Frame::new(f.x + cx, f.y + cy, f.w, f.h);
    let pt = |p: (f32, f32, f32)| (p.0 + cx, p.1 + cy, p.2);
    Layout {
        card: Frame::new(cx, cy, width, h),
        from_icon: (from_icon.0 + cx, from_icon.1 + cy, from_icon.2),
        from: pt(from),
        from_text,
        title,
        title_at: pt(title_at),
        what: at(what),
        what_icon: (what_icon.0 + cx, what_icon.1 + cy, what_icon.2),
        what_label: pt(what_label),
        detail,
        detail_at: pt(detail_at),
        reason_label: reason_label.map(pt),
        reason,
        reason_at: pt(reason_at),
        hints_at: pt(hints_at),
        clock_icon: (clock_icon.0 + cx, clock_icon.1 + cy, clock_icon.2),
        secs_at: at(secs_at),
        bar: at(bar),
    }
}

/// The sheet for `p`: the narrowest width that holds it in the safe area with all its words, else the widest with the reason cut first.
fn layout(kit: &mut Kit, p: &Prompt) -> Layout {
    let whole = Caps { title: 6, detail: 10, reason: 6 };
    for w in WIDTHS {
        let l = plan(kit, p, w, whole);
        if l.card.h <= MAX_H {
            return l;
        }
    }
    let widest = WIDTHS[WIDTHS.len() - 1];
    for reason in [4, 3, 2, 1] {
        let l = plan(kit, p, widest, Caps { reason, ..whole });
        if l.card.h <= MAX_H {
            return l;
        }
    }
    plan(kit, p, widest, Caps { title: 3, detail: 5, reason: 1 })
}

struct Built {
    sig: String,
    lay: Layout,
    _scrim: ShapeLayer,
    _card: Pane,
    _from_icon: Option<ImageLayer>,
    from: Text,
    title: Words,
    _what: ShapeLayer,
    _what_icon: Option<ImageLayer>,
    what_label: Text,
    detail: Words,
    reason_label: Option<Text>,
    reason: Words,
    hints: Vec<HintRow>,
    hint_words: Vec<(String, Frame)>,
    secs_text: Text,
    _clock: Option<ImageLayer>,
    _track: ShapeLayer,
    fill: ShapeLayer,
    /// The seconds the words say now, and the fraction the bar shows.
    secs: u64,
    fraction: f32,
    layers: usize,
}

pub struct AiSurface {
    z: i32,
    root: Option<Root>,
    built: Option<Built>,
    /// The prompt has gone: the sheet fades out and is dropped at this time.
    leaving_until: Option<u64>,
}

impl AiSurface {
    /// `z` is the prompt's depth in the system band.
    pub fn new(z: i32) -> Self {
        Self { z, root: None, built: None, leaving_until: None }
    }

    /// Whether anything of the prompt is on the screen.
    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    fn signature(kit: &Kit, p: &Prompt, over_video: bool) -> String {
        format!("{}|{}|{:?}|{}", kit.generation(), kit.text_scale, (&p.id, &p.kind, &p.who, &p.title, &p.detail, &p.reason), over_video)
    }

    fn build(kit: &mut Kit, root: &Root, z: i32, p: &Prompt, over_video: bool, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let lay = layout(kit, p);
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let scrim = kit.ui.look.colour(PaletteRole::Scrim);
        let wash = faded(ink, kit.tokens.states.selected_wash);
        let (track_fill, bar_fill) = (kit.ui.look.colour(PaletteRole::TrackOff), kit.ui.look.accent());
        scene.adopting_view(&root.group, || -> std::io::Result<Built> {
                let dim = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(scrim, 0.0));
                let card = Pane::new(kit, lay.card, z + 1, PaneKind::Sheet, over_video);
                let from_icon = icon::layer(kit, "sparkles", lay.from_icon.2, kit.ui.look.accent(), z + 2, Props::at(lay.from_icon.0, lay.from_icon.1))?;
                let caption = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left);
                let from = Text::new(kit, Frame::new(lay.from.0, lay.from.1, lay.from.2, kit.line(TypeRole::Caption)), z + 2, caption, &lay.from_text)?;
                let title_lines: Vec<WordLine> = lay.title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
                let title = Words::new(kit, lay.title_at.0, lay.title_at.1, lay.title_at.2, z + 2, &title_lines)?;
                let shape = Shape { corner_radius: kit.tokens.radius(Radius::Control), fill: Fill::Solid(wash), border_width: 1.0, border: edge(kit) };
                let what = plate::shape(&kit.ui, lay.what, z + 2, shape);
                let what_icon = icon::layer(kit, p.icon(), lay.what_icon.2, ink, z + 3, Props::at(lay.what_icon.0, lay.what_icon.1))?;
                let what_label = Text::new(kit, Frame::new(lay.what_label.0, lay.what_label.1, lay.what_label.2, kit.line(TypeRole::Caption)), z + 3, caption, "What will happen")?;
                let detail_lines: Vec<WordLine> = lay.detail.iter().map(|t| WordLine::new(t, TypeRole::Body, ink)).collect();
                let detail = Words::new(kit, lay.detail_at.0, lay.detail_at.1, lay.detail_at.2, z + 3, &detail_lines)?;
                let reason_label = match lay.reason_label {
                    Some(at) => Some(Text::new(kit, Frame::new(at.0, at.1, at.2, kit.line(TypeRole::Caption)), z + 2, caption, &p.reason_label())?),
                    None => None,
                };
                let reason_lines: Vec<WordLine> = lay.reason.iter().map(|t| WordLine::new(t, TypeRole::Body, ink2)).collect();
                let reason = Words::new(kit, lay.reason_at.0, lay.reason_at.1, lay.reason_at.2, z + 2, &reason_lines)?;
                // the keys: what each does, in glyphs; the physical remote's keys are the answer, nothing here is
                let mut rows = Vec::new();
                let mut hint_words = Vec::new();
                let mut x = lay.hints_at.0;
                let gap = kit.space(7);
                let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
                for h in hints() {
                    let row = HintRow::new(kit, &h, TypeRole::Label, RowAt { x, y: lay.hints_at.1, h: lay.hints_at.2, z: z + 2 }, ink)?;
                    if let Some(label) = &h.label {
                        let (w, line) = (width_of(kit, label_style, label), kit.line(TypeRole::Label));
                        hint_words.push((label.clone(), Frame::new(x + row.width() - w, lay.hints_at.1 + ((lay.hints_at.2 - line) / 2.0).round(), w, line)));
                    }
                    x += row.width() + gap;
                    rows.push(row);
                }
                let secs_text = Text::new(kit, lay.secs_at, z + 2, role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Right), "")?;
                let clock = icon::layer(kit, "timer", lay.clock_icon.2, ink2, z + 2, Props::at(lay.clock_icon.0, lay.clock_icon.1))?;
                let track = plate::shape(&kit.ui, lay.bar, z + 2, plate::solid(track_fill, lay.bar.h / 2.0));
                let fill = plate::shape(&kit.ui, lay.bar, z + 3, plate::solid(bar_fill, lay.bar.h / 2.0));
                let layers = before.saturating_sub(kit.room().layers);
                Ok(Built {
                    sig,
                    lay: lay.clone(),
                    _scrim: dim,
                    _card: card,
                    _from_icon: from_icon,
                    from,
                    title,
                    _what: what,
                    _what_icon: what_icon,
                    what_label,
                    detail,
                    reason_label,
                    reason,
                    hints: rows,
                    hint_words,
                    secs_text,
                    _clock: clock,
                    _track: track,
                    fill,
                    secs: u64::MAX,
                    fraction: -1.0,
                    layers,
                })
            })
    }

    /// Fits the screen to the view: the sheet is made for a prompt that came, its countdown follows the clock, and a prompt that went fades out.
    /// Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, view: Option<&View>, over_video: bool, now_ms: u64) -> std::io::Result<bool> {
        let Some(v) = view else {
            if self.root.is_some() && self.leaving_until.is_none() {
                if let Some(root) = self.root.as_mut() {
                    root.hide(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
                }
                self.leaving_until = Some(now_ms + u64::from(FADE_MS) + 60);
            }
            if self.leaving_until.is_some_and(|t| now_ms >= t) {
                self.drop_all();
            }
            return Ok(self.root.is_some());
        };
        let sig = Self::signature(kit, &v.prompt, over_video);
        if self.built.as_ref().is_some_and(|b| b.sig != sig) {
            self.drop_all();
        }
        self.leaving_until = None;
        if self.root.is_none() {
            self.root = Some(Root::new(kit, Arrive::Fade, false)?);
        }
        if self.built.is_none() {
            let root = self.root.as_ref().expect("a root");
            let built = Self::build(kit, root, self.z, &v.prompt, over_video, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the AI prompt could not be drawn whole"));
            }
            self.built = Some(built);
        }
        if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
            root.show(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
        }
        if let Some(b) = self.built.as_mut() {
            if b.secs != v.secs_left {
                b.secs = v.secs_left;
                b.secs_text.set(kit, &format!("{} s", v.secs_left));
            }
            if (b.fraction - v.fraction).abs() > 0.002 {
                b.fraction = v.fraction;
                let f = b.lay.bar;
                let clip = Rect { x: f.x, y: f.y, w: (f.w * v.fraction).round(), h: f.h };
                b.fill.set(Props { clip: Some(clip), visible: v.fraction > 0.0, ..Props::at(f.x, f.y) });
            }
        }
        Ok(true)
    }

    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.leaving_until.is_some_and(|t| now_ms >= t) {
            self.drop_all();
            return true;
        }
        false
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving_until
    }

    fn drop_all(&mut self) {
        self.built = None;
        self.root = None;
        self.leaving_until = None;
    }

    pub fn restyle(&mut self) {
        self.drop_all();
    }

    /// How many layers the sheet holds.
    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers)
    }

    pub fn geometry(&self) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        json!({
            "card": f(b.lay.card),
            "what": f(b.lay.what),
            "title": b.lay.title,
            "detail": b.lay.detail,
            "reason": b.lay.reason,
            "hints": b.hints.iter().map(|h| h.name().to_string()).collect::<Vec<_>>(),
            "secs": b.secs_text.text(),
            "bar": f(b.lay.bar),
            "fraction": b.fraction,
            "layers": self.layers(),
        })
    }

    /// Describes what is on the screen to a lint: the sheet, its words, the key hints' labels and the countdown.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        let card = lint.panel("ai card", b.lay.card, paint, true);
        let caption = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), kit.ui.look.ink2(), Align::Left);
        lint.word(kit, "ai from", b.from.text(), b.from.frame(), caption, Some(card));
        b.title.feed_lint(kit, lint, "ai title", Some(card), (0.0, 0.0));
        let wash = faded(kit.ui.look.ink(), kit.tokens.states.selected_wash);
        let what = lint.panel_on("ai what", b.lay.what, wash, false, card);
        lint.word(kit, "ai what label", b.what_label.text(), b.what_label.frame(), caption, Some(what));
        b.detail.feed_lint(kit, lint, "ai detail", Some(what), (0.0, 0.0));
        if let Some(l) = &b.reason_label {
            lint.word(kit, "ai reason label", l.text(), l.frame(), caption, Some(card));
        }
        b.reason.feed_lint(kit, lint, "ai reason", Some(card), (0.0, 0.0));
        let label_style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Left);
        for (text, frame) in &b.hint_words {
            lint.word(kit, "ai key hint", text, *frame, label_style, Some(card));
        }
        let right = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), kit.ui.look.ink2(), Align::Right);
        lint.word(kit, "ai seconds", b.secs_text.text(), b.secs_text.frame(), right, Some(card));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::system::ai::AiPrompt;
    use crate::kit::screen::Input;
    use ui::lucent::palette::Mode;

    const T0: u64 = 1_800_000_000_000;

    fn prompt_doc(detail: &str, reason: &str, who: Option<&str>) -> Value {
        json!({"id": "a1b2c3", "kind": "tv_launch_app", "assistant": who.map(|w| json!({"id": "x", "name": w})), "title": "Open Netflix",
               "detail": detail, "reason": reason, "shownAt": T0, "expiresAt": T0 + 30_000, "allow": "OK", "deny": "Back"})
    }

    fn show(a: &mut AiPrompt, doc: Value) {
        a.update(&Input::Helper { topic: crate::system::ai::TOPIC.into(), data: doc }, T0);
    }

    fn lint_of(kit: &mut Kit, s: &AiSurface) -> Vec<String> {
        let mut lint = Lint::new();
        s.feed_lint(kit, &mut lint);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_sheet_holds_the_helpers_words_whole_at_both_text_sizes_in_both_materials_and_reads_clean() {
        let detail = "Cody wants to open Netflix now, which takes whatever is on the screen away until someone comes back to it.";
        let reason = "Nitin asked for a movie tonight and said I could start whatever he was watching last on this TV.";
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut model = AiPrompt::new();
                show(&mut model, prompt_doc(detail, reason, Some("Cody")));
                let mut s = AiSurface::new(1000);
                assert!(!s.is_up());
                assert!(s.apply(&mut kit, model.view(T0 + 100).as_ref(), false, T0 + 100).unwrap());
                let g = s.geometry();
                let card = &g["card"];
                let (x, w, y, h) = (card["x"].as_f64().unwrap(), card["w"].as_f64().unwrap(), card["y"].as_f64().unwrap(), card["h"].as_f64().unwrap());
                assert_eq!(x + w / 2.0, 960.0, "in the middle: {g}");
                assert!(y >= 54.0 - 0.5 && y + h <= 1080.0 - 54.0 + 0.5, "{mode:?} at {scale}: in the safe area: {g}");
                assert_eq!(g["hints"], json!(["OK, Allow once", "Back, Deny"]), "the keys as glyphs with what they do");
                let said = |k: &str| g[k].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect::<Vec<_>>().join(" ");
                assert_eq!(said("detail"), detail, "{mode:?} at {scale}: the helper's own words are whole");
                assert_eq!(said("title"), "Open Netflix");
                assert_eq!(said("reason"), format!("\u{201c}{reason}\u{201d}"), "{mode:?} at {scale}: and the reason fits the room it has");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                if scale > 1.5 {
                    assert!(w > 880.0, "the sheet is wider for large text so the words stay whole: {g}");
                }
            }
        }
    }

    #[test]
    fn a_reason_too_long_for_the_safe_area_is_cut_before_anything_of_the_helpers_is() {
        let detail = "Cody wants to open Netflix now, which takes whatever is on the screen away until someone comes back to it.";
        let reason = "Nitin asked for a movie tonight and said I could start whatever he was watching last on this TV, so I am asking to open it for him from the kitchen, because he is cooking and cannot reach the remote just now. ".repeat(3);
        let (_h, mut kit) = testkit::kit(2.0);
        let mut model = AiPrompt::new();
        show(&mut model, prompt_doc(detail, &reason, Some("Cody")));
        let mut s = AiSurface::new(1000);
        s.apply(&mut kit, model.view(T0).as_ref(), false, T0).unwrap();
        let g = s.geometry();
        let said = |k: &str| g[k].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect::<Vec<_>>().join(" ");
        assert_eq!(said("detail"), detail, "the helper's words are whole: {g}");
        assert!(said("reason").ends_with('\u{2026}'), "the reason is cut and says so: {}", said("reason"));
        let (y, h) = (g["card"]["y"].as_f64().unwrap(), g["card"]["h"].as_f64().unwrap());
        assert!(y >= 54.0 - 0.5 && y + h <= 1080.0 - 54.0 + 0.5, "still in the safe area: {g}");
        assert!(lint_of(&mut kit, &s).is_empty());
    }

    #[test]
    fn the_countdown_follows_the_clock_the_bar_empties_and_a_prompt_that_goes_fades_out_and_is_dropped() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut model = AiPrompt::new();
        show(&mut model, prompt_doc("The TV opens Netflix.", "", None));
        let mut s = AiSurface::new(1000);
        s.apply(&mut kit, model.view(T0).as_ref(), false, T0).unwrap();
        assert_eq!(s.geometry()["secs"], "30 s");
        assert_eq!(s.geometry()["fraction"].as_f64().unwrap(), 1.0);
        s.apply(&mut kit, model.view(T0 + 15_000).as_ref(), false, T0 + 15_000).unwrap();
        assert_eq!(s.geometry()["secs"], "15 s");
        assert_eq!(s.geometry()["fraction"].as_f64().unwrap(), 0.5);
        let layers = s.layers();
        assert!(layers > 0 && layers <= 24, "a lean sheet: {layers} layers");
        // the helper takes the topic away: it fades out, then nothing is kept
        assert!(s.apply(&mut kit, None, false, T0 + 16_000).unwrap(), "still fading");
        assert!(s.due_at().is_some());
        assert!(s.tick(T0 + 17_000));
        assert!(!s.is_up() && s.layers() == 0, "nothing hidden is kept");
        assert!(!s.apply(&mut kit, None, false, T0 + 18_000).unwrap());
    }

    #[test]
    fn it_is_drawn_again_for_another_prompt_and_without_a_reason_it_has_none_to_show() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut model = AiPrompt::new();
        show(&mut model, prompt_doc("The TV opens Netflix.", "", None));
        let mut s = AiSurface::new(1000);
        s.apply(&mut kit, model.view(T0).as_ref(), false, T0).unwrap();
        let without = s.geometry()["card"]["h"].as_f64().unwrap();
        show(&mut model, prompt_doc("The TV opens Netflix.", "Nitin asked for a movie", Some("Cody")));
        s.apply(&mut kit, model.view(T0).as_ref(), false, T0).unwrap();
        assert!(s.geometry()["card"]["h"].as_f64().unwrap() > without, "the reason has its place under the words, labelled");
    }
}
