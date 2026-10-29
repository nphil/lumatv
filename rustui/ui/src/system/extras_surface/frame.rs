//! The mini frame drawn (`MiniFrame.qml`): the pane a news story opens in, 1520 by 860 under the status bar, that grows out of its card, with a
//! bar (the story's title, its source and age at the right, a hairline) and the room below it left clear for the web half (`<app>.frame`) to paint
//! into, 32 in from the sides and the foot. The pane grows as one group from the card to its place; the words and the loading ring come once it has
//! landed (the model says so), and the ring goes when the web half is up. Closing is the same the other way round.

use super::spin_head;
use super::spinner::Spinner;
use crate::kit::Kit;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::edge;
use crate::kit::wrap;
use crate::panels::ui::{Arrive, Root};
use crate::system::extras::FrameView;
use lumatv_tokens::TypeRole;
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame as Rect;
use ui::lucent::palette::PaletteRole;
use ui::scene::ShapeLayer;
use ui::text::Weight;

/// The bar's height and the pad round the frame's interior.
const BAR: f32 = 64.0;
const PAD: f32 = 32.0;
const GROW_MS: u32 = 260;
const SHRINK_MS: u32 = 220;
const LAND_FADE_MS: u32 = 160;
const RING: f32 = 64.0;

fn rect(a: [f32; 4]) -> Rect {
    Rect::new(a[0], a[1], a[2], a[3])
}

struct Built {
    sig: String,
    pane: Pane,
    bar_h: f32,
}

struct Words {
    sig: String,
    title: Text,
    sub: Text,
    _rule: ShapeLayer,
    ring: Spinner,
}

pub struct FrameSurface {
    z: i32,
    root: Option<Root>,
    built: Option<Built>,
    words_root: Option<Root>,
    words: Option<Words>,
    leaving_until: Option<u64>,
}

impl FrameSurface {
    pub fn new(z: i32) -> Self {
        Self { z, root: None, built: None, words_root: None, words: None, leaving_until: None }
    }

    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    fn bar_height(kit: &Kit) -> f32 {
        BAR.max(kit.line(TypeRole::Label) + kit.space(3)).ceil()
    }

    fn build_words(kit: &mut Kit, root: &Root, z: i32, v: &FrameView, sig: String) -> std::io::Result<Words> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let to = rect(v.to);
        let bar_h = Self::bar_height(kit);
        let title_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let sub_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
        let (title_h, sub_h) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
        let gap = kit.space(5);
        let inner = to.w - 2.0 * PAD;
        let sub_w = width_of(kit, sub_style, &v.subtitle).min(inner * 0.4);
        let sub_text = wrap::elide(kit, sub_style, &v.subtitle, sub_w);
        let title_w = (inner - sub_w - gap).max(80.0);
        let title_text = wrap::elide(kit, title_style, &v.title, title_w);
        let edge_colour = edge(kit);
        scene.adopting_view(&root.group, || -> std::io::Result<Words> {
            let title = Text::new(kit, Rect::new(to.x + PAD, (to.y + (bar_h - title_h) / 2.0).round(), title_w, title_h), z + 1, title_style, &title_text)?;
            let sub = Text::new(kit, Rect::new(to.right() - PAD - sub_w, (to.y + (bar_h - sub_h) / 2.0).round(), sub_w, sub_h), z + 1, sub_style, &sub_text)?;
            let rule = plate::shape(&kit.ui, Rect::new(to.x + PAD, to.y + bar_h - 1.0, inner, 1.0), z + 1, plate::solid(edge_colour, 0.0));
            let ring = Spinner::new(kit, (to.x + (to.w - RING) / 2.0).round(), (to.y + bar_h + (to.h - bar_h - RING) / 2.0).round(), RING, 7.0, z + 1, ink);
            Ok(Words { sig, title, sub, _rule: rule, ring })
        })
    }

    /// Fits the frame to the view: it grows out of its card, the words come when it has landed and the ring while it loads, and it shrinks back when it
    /// goes. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, view: Option<&FrameView>, now_ms: u64) -> std::io::Result<bool> {
        let Some(v) = view else {
            if self.root.is_some() && self.leaving_until.is_none() {
                if let Some(r) = self.root.as_mut() {
                    r.hide(SHRINK_MS);
                }
                if let Some(r) = self.words_root.as_mut() {
                    r.hide(kit.beat_ms(crate::kit::motion::Beat::Label).min(LAND_FADE_MS));
                }
                self.leaving_until = Some(now_ms + u64::from(SHRINK_MS) + 60);
            }
            if self.leaving_until.is_some_and(|t| now_ms >= t) {
                self.drop_all();
            }
            return Ok(self.root.is_some());
        };
        self.leaving_until = None;
        let sig = format!("{}|{}|{:?}", kit.generation(), kit.text_scale, (v.from.map(|x| x as i32), v.to.map(|x| x as i32)));
        if self.built.as_ref().is_some_and(|b| b.sig != sig) {
            self.drop_all();
        }
        if self.root.is_none() {
            self.root = Some(Root::new(kit, Arrive::Grow { from: v.from, to: v.to }, false)?);
        }
        if self.built.is_none() {
            let root = self.root.as_ref().expect("a root");
            let scene = kit.ui.scene.clone();
            let bar_h = Self::bar_height(kit);
            let pane = scene.adopting_view(&root.group, || Pane::new(kit, rect(v.to), self.z, PaneKind::Card, false));
            self.built = Some(Built { sig, pane, bar_h });
        }
        if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
            root.show(if kit.ui.look.reduced_motion { kit.beat_ms(crate::kit::motion::Beat::Layer) } else { GROW_MS });
        }
        if v.landed {
            let words_sig = format!("{}|{}|{}|{}", kit.generation(), kit.text_scale, v.title, v.subtitle);
            if self.words.as_ref().is_some_and(|w| w.sig != words_sig) {
                self.words = None;
                self.words_root = None;
            }
            if self.words_root.is_none() {
                self.words_root = Some(Root::new(kit, Arrive::Fade, false)?);
            }
            if self.words.is_none() {
                let root = self.words_root.as_ref().expect("a root");
                self.words = Some(Self::build_words(kit, root, self.z + 1, v, words_sig)?);
            }
            if let Some(root) = self.words_root.as_mut().filter(|r| !r.is_shown()) {
                root.show(kit.beat_ms(crate::kit::motion::Beat::Label).min(LAND_FADE_MS));
            }
            let reduced = kit.ui.look.reduced_motion;
            if let Some(w) = self.words.as_mut() {
                w.ring.show(v.loading);
                if v.loading && !reduced {
                    w.ring.step(spin_head(now_ms));
                }
            }
        }
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the mini frame could not be drawn whole"));
        }
        Ok(true)
    }

    /// The ring turns in place; what has shrunk back is dropped. Returns whether a redraw is wanted.
    pub fn tick(&mut self, kit: &Kit, now_ms: u64) -> bool {
        if let Some(w) = self.words.as_mut().filter(|_| !kit.ui.look.reduced_motion) {
            w.ring.step(spin_head(now_ms));
        }
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
        self.words = None;
        self.words_root = None;
        self.built = None;
        self.root = None;
        self.leaving_until = None;
    }

    pub fn restyle(&mut self) {
        self.drop_all();
    }

    pub fn geometry(&self) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = b.pane.frame();
        let interior = json!({"x": f.x + PAD, "y": f.y + b.bar_h, "w": f.w - 2.0 * PAD, "h": f.h - b.bar_h - PAD});
        json!({
            "x": f.x, "y": f.y, "w": f.w, "h": f.h, "interior": interior,
            "title": self.words.as_ref().map(|w| w.title.text().to_string()), "subtitle": self.words.as_ref().map(|w| w.sub.text().to_string()),
            "ring": self.words.as_ref().map(|w| w.ring.frame()).map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})),
        })
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let (Some(b), Some(w)) = (&self.built, &self.words) else { return };
        let panel = lint.panel("mini frame", b.pane.frame(), kit.ui.look.colour(PaletteRole::Sheet), true);
        let title = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Left);
        let sub = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink2(), Align::Right);
        lint.word(kit, "mini frame title", w.title.text(), w.title.frame(), title, Some(panel));
        lint.word(kit, "mini frame subtitle", w.sub.text(), w.sub.frame(), sub, Some(panel));
    }
}
