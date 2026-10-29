//! The launch screen drawn (`panels/Splash.qml`, mode `launch`): the page hidden behind a scrim, the app's icon at 180 with a corner of 38, its
//! name as a title and a ring that turns beside the word "Opening". It is Home's own: it stands over Home and under every panel, and is gone the
//! moment an app has the front. The icon's place is told to the model, which hands it to the compositor: that is where the app's card grows from.

use super::{app_icon, spin_head};
use crate::kit::Kit;
use crate::kit::lint::Lint;
use crate::kit::text::{Text, role_style};
use crate::kit::wrap;
use crate::panels::ui::{Arrive, Root};
use crate::system::extras::{SplashView, Tile};
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::TypeRole;
use serde_json::{Value, json};
use super::spinner::Spinner;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const ICON: f32 = 180.0;
const ICON_RADIUS: f32 = 38.0;
const RING: f32 = 28.0;
const FADE_MS: u32 = 180;
/// The name is cut to this width: a title wider than the screen's safe area would leave it.
const NAME_W: f32 = 1600.0;

struct Built {
    sig: String,
    _scrim: ShapeLayer,
    _icon: ImageLayer,
    name: Text,
    caption: Text,
    ring: Spinner,
    icon_at: Frame,
}

pub struct Splash {
    z: i32,
    root: Option<Root>,
    built: Option<Built>,
}

impl Splash {
    pub fn new(z: i32) -> Self {
        Self { z, root: None, built: None }
    }

    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    /// Where the icon is, with its corner: what the compositor's card grows from.
    pub fn icon_rect(&self) -> Option<Tile> {
        self.built.as_ref().map(|b| Tile { x: b.icon_at.x, y: b.icon_at.y, w: b.icon_at.w, h: b.icon_at.h, radius: ICON_RADIUS })
    }

    fn signature(kit: &Kit, v: &SplashView) -> String {
        format!("{}|{}|{:?}|{}", kit.generation(), kit.text_scale, (&v.name, &v.icon), v.fade)
    }

    fn build(kit: &mut Kit, root: &Root, z: i32, v: &SplashView, sig: String) -> std::io::Result<Built> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let hide = kit.tokens.extra(kit.mode).scrim_hide;
        let (s3, s6) = (kit.space(3), kit.space(6));
        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Centre);
        let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let (title_h, caption_h) = (kit.line(TypeRole::Title), kit.line(TypeRole::Caption));
        let row_h = RING.max(caption_h);
        let total = ICON + s6 + title_h + s6 + row_h;
        let top = ((1080.0 - total) / 2.0).round();
        let icon_at = Frame::new((1920.0 - ICON) / 2.0, top, ICON, ICON);
        let name_text = wrap::elide(kit, title_style, &v.name, NAME_W);
        let caption_w = crate::kit::text::width_of(kit, caption_style, "Opening");
        let row_x = ((1920.0 - (RING + s3 + caption_w)) / 2.0).round();
        let row_y = (top + ICON + s6 + title_h + s6).round();
        scene.adopting_view(&root.group, || -> std::io::Result<Built> {
            let dim = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(hide, 0.0));
            let icon = app_icon(kit, &v.icon, ICON as u32, ICON_RADIUS, z + 1, Props::at(icon_at.x, icon_at.y))?;
            let name = Text::new(kit, Frame::new((1920.0 - NAME_W) / 2.0, top + ICON + s6, NAME_W, title_h), z + 1, title_style, &name_text)?;
            let ring = Spinner::new(kit, row_x, (row_y + (row_h - RING) / 2.0).round(), RING, 4.0, z + 1, ink);
            let caption = Text::new(kit, Frame::new(row_x + RING + s3, (row_y + (row_h - caption_h) / 2.0).round(), caption_w, caption_h), z + 1, caption_style, "Opening")?;
            Ok(Built { sig, _scrim: dim, _icon: icon, name, caption, ring, icon_at })
        })
    }

    /// Fits the screen to the view: made for a launch that came (or whose words, look or size changed), its ring stepped to `now`, and dropped
    /// at once when the launch is over. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, view: Option<&SplashView>, now_ms: u64) -> std::io::Result<bool> {
        let Some(v) = view else {
            self.root = None;
            self.built = None;
            return Ok(false);
        };
        let sig = Self::signature(kit, v);
        if self.built.as_ref().is_some_and(|b| b.sig != sig) {
            self.built = None;
            self.root = None;
        }
        if self.root.is_none() {
            // a screen that came after a wait fades in; one that is there at once is there at once
            self.root = Some(Root::new(kit, Arrive::Fade, !v.fade)?);
        }
        if self.built.is_none() {
            let root = self.root.as_ref().expect("a root");
            let built = Self::build(kit, root, self.z, v, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the launch screen could not be drawn whole"));
            }
            self.built = Some(built);
        }
        if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
            root.show(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
        }
        let reduced = kit.ui.look.reduced_motion;
        if let Some(b) = self.built.as_mut() {
            b.ring.step(if reduced { 0 } else { spin_head(now_ms) });
        }
        Ok(true)
    }

    /// The ring steps in place: no layer is made, so no redraw is asked for.
    pub fn tick(&mut self, now_ms: u64, reduced: bool) {
        if let Some(b) = self.built.as_mut().filter(|_| !reduced) {
            b.ring.step(spin_head(now_ms));
        }
    }

    pub fn restyle(&mut self) {
        self.built = None;
        self.root = None;
    }

    pub fn geometry(&self) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        json!({"icon": f(b.icon_at), "radius": ICON_RADIUS, "name": b.name.text(), "caption": b.caption.text(), "ring": f(b.ring.frame()), "head": b.ring.head()})
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let paint: Rgba = kit.tokens.extra(kit.mode).scrim_hide;
        let panel = lint.panel("launch screen", Frame::new(0.0, 0.0, 1920.0, 1080.0), paint, false);
        let title = role_style(kit, TypeRole::Title, None, kit.ui.look.ink(), Align::Centre);
        lint.word(kit, "launch name", b.name.text(), b.name.frame(), title, Some(panel));
        let caption = role_style(kit, TypeRole::Caption, Some(Weight::Regular), kit.ui.look.ink2(), Align::Left);
        lint.word(kit, "launch caption", b.caption.text(), b.caption.frame(), caption, Some(panel));
    }
}
