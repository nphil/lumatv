//! The Cameras app's surface: the base (the grid or the full view), what is over it (the strip, the sheet, the options page, a refusal's
//! words) and the pictures, kept in step with the model's [`View`]. Everything is built when the app opens and dropped when it closes;
//! a piece is made again only when what it says changes, and what leaves the screen fades out over the chrome's beat and is dropped.

use super::full::{FADE_OUT_MS, FullUi};
use super::grid::GridUi;
use super::options::OptionsUi;
use super::pic::PictureStore;
use super::sheet::SheetUi;
use super::strip::StripUi;
use super::toast::ToastUi;
use super::{Cx, ids};
use crate::cameras::app::Target;
use crate::cameras::view::{Base, View};
use crate::kit::focus::Id;
use crate::kit::lint::Lint;
use crate::kit::picture::Picture;
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::{Engine, Kit};
use serde_json::{Value, json};
use ui::geometry::Frame;

/// What is fading out: held until its time is up (a layer goes with its owner).
type Leaving = Box<dyn std::any::Any>;

pub struct CamerasSurface {
    z: i32,
    store: PictureStore,
    pointer: Pointer,
    grid: Option<GridUi>,
    full: Option<FullUi>,
    strip: Option<StripUi>,
    sheet: Option<SheetUi>,
    options: Option<OptionsUi>,
    toast: Option<ToastUi>,
    leaving: Vec<(Leaving, u64)>,
    /// Builds and rebuilds since the start (the harness reads it).
    pub builds: u32,
}

impl CamerasSurface {
    /// `z` is the band's base: `system_z + kit::z::CAMERAS`.
    pub fn new(z: i32) -> Self {
        Self {
            z,
            store: PictureStore::new(),
            pointer: Pointer::new(),
            grid: None,
            full: None,
            strip: None,
            sheet: None,
            options: None,
            toast: None,
            leaving: Vec::new(),
            builds: 0,
        }
    }

    pub fn store(&self) -> &PictureStore {
        &self.store
    }

    pub fn has_picture(&self, key: &str) -> bool {
        self.store.has(key)
    }

    /// A decoded picture arrived for `key`.
    pub fn put_picture(&mut self, key: &str, pic: Picture) {
        self.store.put(key, pic);
    }

    /// The model let go of these keys (a key ending in `:` is a prefix).
    pub fn forget(&mut self, keys: &[String]) {
        for k in keys {
            self.store.forget(k);
        }
    }

    pub fn is_up(&self) -> bool {
        self.grid.is_some() || self.full.is_some()
    }

    /// Whether anything of the app is on the screen (including what is fading out).
    pub fn is_live(&self) -> bool {
        self.is_up() || !self.leaving.is_empty()
    }

    /// Takes everything off the screen at once (the app closed, the look changed, the screen went dark).
    pub fn close_all(&mut self, engine: &mut Engine) {
        if let Some(g) = self.grid.take() {
            g.retire(engine);
        }
        if let Some(f) = self.full.take() {
            f.retire(engine);
        }
        self.drop_overlays(engine);
        self.leaving.clear();
    }

    fn drop_overlays(&mut self, engine: &mut Engine) {
        // the scopes close newest first
        if let Some(mut o) = self.options.take() {
            Self::retire_options(&mut o, engine);
        }
        if let Some(mut s) = self.sheet.take() {
            Self::retire_sheet(&mut s, engine);
        }
        if let Some(mut s) = self.strip.take() {
            Self::retire_strip(&mut s, engine);
        }
        self.toast = None;
    }

    // `retire` needs a Cx for the kit's fade; at once (0 ms) it needs the engine only: these do the engine's part.
    fn retire_options(o: &mut OptionsUi, engine: &mut Engine) {
        o.retire_engine(engine);
    }

    fn retire_sheet(s: &mut SheetUi, engine: &mut Engine) {
        s.retire_engine(engine);
    }

    fn retire_strip(s: &mut StripUi, engine: &mut Engine) {
        s.retire_engine(engine);
    }

    /// Draws `view` at `now_ms`. Returns whether anything of the app is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ms: u64) -> std::io::Result<bool> {
        if !view.is_up() {
            if self.is_up() || self.strip.is_some() || self.sheet.is_some() || self.options.is_some() {
                self.close_all(engine);
            }
            return Ok(!self.leaving.is_empty());
        }
        // what is no longer wanted leaves first: the scopes close newest first, then the new ones open
        self.retire_unwanted(kit, engine, view, now_ms);
        // the base
        match &view.base {
            Base::Grid(g) => {
                if let Some(f) = self.full.take() {
                    f.retire(engine);
                }
                let stale = self.grid.as_ref().is_some_and(|ui| !ui.fits(&Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 }, g));
                if stale {
                    if let Some(old) = self.grid.take() {
                        old.retire(engine);
                    }
                }
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 };
                cx.now_ns = cx.kit.ui.scene.now_ns();
                match &mut self.grid {
                    Some(ui) => ui.sync(&mut cx, g)?,
                    None => {
                        self.grid = Some(GridUi::build(&mut cx, g)?);
                        self.builds += 1;
                    }
                }
            }
            Base::Full(f) => {
                if let Some(g) = self.grid.take() {
                    g.retire(engine);
                }
                let stale = self.full.as_ref().is_some_and(|ui| !ui.fits(&Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 }, f));
                if stale {
                    if let Some(old) = self.full.take() {
                        old.retire(engine);
                    }
                }
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 };
                cx.now_ns = cx.kit.ui.scene.now_ns();
                match &mut self.full {
                    Some(ui) => ui.sync(&mut cx, f)?,
                    None => {
                        self.full = Some(FullUi::build(&mut cx, f)?);
                        self.builds += 1;
                    }
                }
            }
            Base::Closed => {}
        }
        self.sync_overlays(kit, engine, view, now_ms)?;
        Ok(true)
    }

    /// Retires the overlays the view no longer has (with the chrome's fade), or whose look is stale.
    fn retire_unwanted(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ms: u64) {
        let over_video = matches!(view.base, Base::Full(_));
        let until = now_ms + u64::from(FADE_OUT_MS) + 40;
        let ms = FADE_OUT_MS;
        if self.options.is_some() && view.options.is_none() || self.options.as_ref().zip(view.options.as_ref()).is_some_and(|(u, v)| !u.fits(&Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 }, v)) {
            if let Some(mut o) = self.options.take() {
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 };
                o.retire(&mut cx, ms);
                self.leaving.push((Box::new(o), until));
            }
        }
        if self.sheet.is_some() && view.sheet.is_none() || self.sheet.as_ref().zip(view.sheet.as_ref()).is_some_and(|(u, v)| !u.fits(&Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 }, v, over_video)) {
            if let Some(mut s) = self.sheet.take() {
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 };
                s.retire(&mut cx, ms);
                self.leaving.push((Box::new(s), until));
            }
        }
        if self.strip.is_some() && view.strip.is_none() || self.strip.as_ref().is_some_and(|u| !u.fits(&Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 })) {
            if let Some(mut s) = self.strip.take() {
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 };
                s.retire(&mut cx, ms);
                self.leaving.push((Box::new(s), until));
            }
        }
        if self.toast.is_some() && view.toast.is_none() {
            if let Some(mut t) = self.toast.take() {
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns: 0 };
                t.retire(&mut cx, ms);
                self.leaving.push((Box::new(t), until));
            }
        }
    }

    fn sync_overlays(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ms: u64) -> std::io::Result<()> {
        let over_video = matches!(view.base, Base::Full(_));
        let now_ns = kit.ui.scene.now_ns();
        if let Some(v) = &view.strip {
            let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns };
            match &mut self.strip {
                Some(ui) => ui.sync(&mut cx, v)?,
                None => {
                    self.strip = Some(StripUi::build(&mut cx, v)?);
                    self.builds += 1;
                }
            }
        }
        if let Some(v) = &view.sheet {
            let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns };
            match &mut self.sheet {
                Some(ui) => ui.sync(&mut cx, v)?,
                None => {
                    self.sheet = Some(SheetUi::build(&mut cx, v, over_video)?);
                    self.builds += 1;
                }
            }
        }
        if let Some(v) = &view.options {
            let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns };
            match &mut self.options {
                Some(ui) => ui.sync(&mut cx, v)?,
                None => {
                    self.options = Some(OptionsUi::build(&mut cx, v)?);
                    self.builds += 1;
                }
            }
        }
        if let Some(text) = &view.toast {
            let above = self.strip.as_ref().map(|s| s.clear_top(kit));
            if self.toast.as_ref().is_none_or(|t| &t.text != text || t.above != above) {
                if let Some(mut old) = self.toast.take() {
                    let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns };
                    old.retire(&mut cx, 0);
                }
                let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms, now_ns };
                self.toast = Some(ToastUi::build(&mut cx, text, above)?);
            }
        }
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the cameras could not be drawn whole"));
        }
        Ok(())
    }

    /// Drops what has finished fading out. Returns whether anything went.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let n = self.leaving.len();
        self.leaving.retain(|(_, until)| *until > now_ms);
        n != self.leaving.len()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving.iter().map(|(_, until)| *until).min()
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &mut Kit, engine: &mut Engine) {
        self.pointer.key(engine);
        let ids: Vec<Id> = self.hoverable();
        let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms: 0, now_ns: 0 };
        for id in ids {
            Self::hover_all(&mut self.grid, &mut self.full, &mut self.strip, &mut self.sheet, &mut self.options, &mut cx, id, false);
        }
    }

    fn hoverable(&self) -> Vec<Id> {
        let mut v = vec![ids::HEADER_EVENTS, ids::HEADER_SETTINGS, ids::EMPTY_SETTINGS, ids::RETRY, ids::TAB_EVENTS, ids::TAB_CLIPS, ids::OPTIONS_BACK];
        v.extend([ids::STRIP_SPAN, ids::STRIP_RATE, ids::STRIP_LIVE, ids::STRIP_BACK, ids::STRIP_PLAY, ids::STRIP_FORWARD, ids::STRIP_PREV, ids::STRIP_NEXT]);
        v.extend((0..32).map(ids::tile));
        v.extend((0..32).map(ids::card));
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn hover_all(grid: &mut Option<GridUi>, full: &mut Option<FullUi>, strip: &mut Option<StripUi>, sheet: &mut Option<SheetUi>, options: &mut Option<OptionsUi>, cx: &mut Cx, id: Id, on: bool) {
        if let Some(g) = grid {
            g.hover(cx, id, on);
        }
        if let Some(f) = full {
            f.hover(cx, id, on);
        }
        if let Some(s) = strip {
            s.hover(cx, id, on);
        }
        if let Some(s) = sheet {
            s.hover(cx, id, on);
        }
        if let Some(o) = options {
            o.hover(cx, id, on);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn press_all(grid: &mut Option<GridUi>, full: &mut Option<FullUi>, strip: &mut Option<StripUi>, sheet: &mut Option<SheetUi>, options: &mut Option<OptionsUi>, cx: &mut Cx, id: Id, down: bool) {
        if let Some(g) = grid {
            g.press(cx, id, down);
        }
        if let Some(f) = full {
            f.press(cx, id, down);
        }
        if let Some(s) = strip {
            s.press(cx, id, down);
        }
        if let Some(s) = sheet {
            s.press(cx, id, down);
        }
        if let Some(o) = options {
            o.press(cx, id, down);
        }
    }

    /// The model's target for the focusable `id` clicked at `x`.
    fn target(&self, id: Id, x: f32) -> Option<Target> {
        self.options
            .as_ref()
            .and_then(|o| o.target(id))
            .or_else(|| self.sheet.as_ref().and_then(|s| s.target(id)))
            .or_else(|| self.strip.as_ref().and_then(|s| s.target(id, x)))
            .or_else(|| self.full.as_ref().and_then(|f| f.target(id)))
            .or_else(|| self.grid.as_ref().and_then(|g| g.target(id)))
    }

    /// A sample of the pointer: the cue follows it, a press shrinks the button under it, a click is the target to act on.
    pub fn pointer(&mut self, kit: &mut Kit, engine: &mut Engine, now_ns: u64, sample: Sample) -> Option<Target> {
        if !self.is_up() {
            return None;
        }
        let gestures = self.pointer.feed(engine, now_ns, sample);
        let mut click = None;
        let mut cx = Cx { kit, engine, store: &self.store, z: self.z, now_ms: 0, now_ns };
        for g in gestures {
            match g {
                Gesture::Hover { from, to } => {
                    if let Some(id) = from {
                        Self::hover_all(&mut self.grid, &mut self.full, &mut self.strip, &mut self.sheet, &mut self.options, &mut cx, id, false);
                    }
                    if let Some(id) = to {
                        Self::hover_all(&mut self.grid, &mut self.full, &mut self.strip, &mut self.sheet, &mut self.options, &mut cx, id, true);
                    }
                }
                Gesture::Down(id) => Self::press_all(&mut self.grid, &mut self.full, &mut self.strip, &mut self.sheet, &mut self.options, &mut cx, id, true),
                Gesture::Up(Some(id)) => Self::press_all(&mut self.grid, &mut self.full, &mut self.strip, &mut self.sheet, &mut self.options, &mut cx, id, false),
                Gesture::Up(None) => {}
                Gesture::Click(id) => click = self.target(id, sample.x),
            }
        }
        click
    }

    /// The layers the app holds now, for the budget.
    pub fn layers(&self) -> usize {
        self.grid.as_ref().map_or(0, GridUi::layers)
            + self.full.as_ref().map_or(0, FullUi::layers)
            + self.strip.as_ref().map_or(0, StripUi::layers)
            + self.sheet.as_ref().map_or(0, SheetUi::layers)
            + self.options.as_ref().map_or(0, OptionsUi::layers)
            + self.toast.as_ref().map_or(0, ToastUi::layers)
    }

    /// Where the parts are, for the harness (`--eval geometry`).
    pub fn geometry(&self) -> Value {
        let rect = |f: Frame| json!({"x": f.x, "y": f.y, "w": f.w, "h": f.h});
        json!({
            "base": if self.grid.is_some() { "grid" } else if self.full.is_some() { "full" } else { "" },
            "tiles": self.grid.as_ref().map(|g| g.tile_rects().into_iter().map(rect).collect::<Vec<_>>()),
            "tileCameras": self.grid.as_ref().map(|g| g.tile_cameras().into_iter().map(str::to_string).collect::<Vec<_>>()),
            "pills": self.full.as_ref().map(|f| f.rects().into_iter().map(|(n, r)| (n.to_string(), rect(r))).collect::<std::collections::BTreeMap<_, _>>()),
            "chrome": self.full.as_ref().map(FullUi::chrome_on),
            "strip": self.strip.as_ref().map(|s| rect(s.frame())),
            "sheet": self.sheet.as_ref().map(|s| rect(s.frame())),
            "cards": self.sheet.as_ref().map(|s| s.card_rects().into_iter().map(rect).collect::<Vec<_>>()),
            "cardPictures": self.sheet.as_ref().map(|s| s.painted()),
            "options": self.options.as_ref().map(|o| rect(o.frame())),
            "toast": self.toast.as_ref().map(|t| rect(t.frame())),
            "layers": self.layers(),
            "pictures": {"kept": self.store.len(), "bytes": self.store.bytes(), "keys": self.store.keys()},
            "builds": self.builds,
        })
    }

    /// Describes what is on the screen to a lint.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        if let Some(g) = self.grid.as_ref().filter(|_| self.options.is_none() && self.sheet.is_none()) {
            // under a sheet's or the options page's veil the grid is a hint of where it came from, not something to read
            let scrim = lint.panel("scrim", Frame::new(0.0, 0.0, 1920.0, 1080.0), g.scrim_paint(kit), false);
            g.feed_lint(kit, lint, scrim);
        }
        if let Some(f) = &self.full {
            f.feed_lint(kit, lint);
        }
        if let Some(s) = &self.strip {
            s.feed_lint(kit, lint);
        }
        if let Some(s) = &self.sheet {
            s.feed_lint(kit, lint);
        }
        if let Some(o) = &self.options {
            o.feed_lint(kit, lint);
        }
        if let Some(t) = &self.toast {
            t.feed_lint(kit, lint);
        }
    }

    /// The look or the text size changed: everything is made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.close_all(engine);
    }

}
