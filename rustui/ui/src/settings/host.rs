//! Settings as one part of the shell: the model fed from the channel, the helper, the keys and the clock, the surface drawn with the kit in
//! Settings' band of the engine's z, and what Settings asks of the shell (to close, to launch LG's Settings app, to open the layout editor,
//! to ask for text). Nothing here does I/O: effects go to the shell, which hands them to the host.

use super::model::{Model, Target};
use super::pages;
use super::row::ShellRequest;
use super::surface::SettingsSurface;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input, Model as ScreenModel};
use crate::kit::{Engine, Kit};
use serde_json::{Value, json};

/// The helper's topics Settings reads besides the shell's own (`kit/Helper.qml` `subscribed`).
pub const HELPER_TOPICS: &str = "profiles,openapps,updateblock,panel,receiver,ai,ai.prompt,ha.link,plugins,screen,cameras";

pub struct SettingsHost {
    pub model: Model,
    surface: SettingsSurface,
    redraw: bool,
}

impl SettingsHost {
    /// `system_z` is where stack 1 begins; `theme` the merged theme and `layout` Home's layout.
    pub fn new(system_z: i32, theme: &Value, layout: &Value) -> Self {
        SettingsHost { model: Model::new(pages::all(), theme.clone(), layout.clone()), surface: SettingsSurface::new(system_z), redraw: false }
    }

    /// Describes what is on screen to the layout lint (`--eval lint`).
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint) {
        self.surface.feed_lint(kit, lint);
    }

    pub fn is_open(&self) -> bool {
        self.model.is_open()
    }

    /// Whether Settings wants the keys: while it is open (or fading out, which takes none).
    pub fn wants(&self) -> bool {
        self.model.is_open()
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// Opens Settings, at `page` when one is named (a tile of the quick panel, a card of Home).
    pub fn open(&mut self, page: Option<&str>, now: u64) -> Vec<Effect> {
        self.redraw = true;
        self.model.open(page, now)
    }

    pub fn close(&mut self, now: u64) -> Vec<Effect> {
        self.redraw = true;
        self.model.close(now)
    }

    /// An input of the host: topics and answers keep the model's stores warm while Settings is closed; keys and time matter while it
    /// is open.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let effects = self.model.update(input, now);
        if effects.iter().any(|e| matches!(e, Effect::Redraw)) {
            self.redraw = true;
        }
        effects
    }

    /// A sample of the pointer: the cue follows it and a click is what OK would be.
    pub fn pointer(&mut self, engine: &mut Engine, now_ns: u64, s: Sample, now: u64) -> Vec<Effect> {
        if !self.model.is_open() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let before = self.surface.hovered();
        for t in self.surface.pointer(engine, now_ns, s.x, s.y, s.buttons, s.visible) {
            out.extend(self.click(t, now));
        }
        if self.surface.hovered() != before {
            self.redraw = true;
        }
        out
    }

    pub fn click(&mut self, target: Target, now: u64) -> Vec<Effect> {
        self.redraw = true;
        self.model.click(target, now)
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn key_taken(&mut self, engine: &mut Engine) {
        if self.model.is_open() {
            self.surface.key(engine);
        }
    }

    /// What Settings asked the shell for since the last call.
    pub fn take_requests(&mut self) -> Vec<ShellRequest> {
        self.model.take_requests()
    }

    #[cfg(test)]
    pub fn raise(&mut self, r: ShellRequest) {
        self.model.raise(r);
    }

    /// The theme file after a patch was written (the shell makes it the look in force).
    pub fn take_theme_file(&mut self) -> Option<Value> {
        self.model.take_theme_file()
    }

    pub fn set_theme(&mut self, theme: Value) {
        self.model.set_theme(theme);
        self.redraw = true;
    }

    pub fn set_layout(&mut self, layout: Value) {
        self.model.set_layout(layout);
        self.redraw = true;
    }

    /// The text the person typed for a row that asked for it.
    pub fn entered(&mut self, text: &str, now: u64) -> Vec<Effect> {
        self.redraw = true;
        self.model.entered(text, now)
    }

    pub fn entry_cancelled(&mut self) {
        self.model.entry_cancelled();
        self.redraw = true;
    }

    /// Draws what changed. `over_video` is whether an app has the screen under it. Returns whether any of Settings is on screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, over_video: bool, now: u64, now_ns: u64) -> std::io::Result<bool> {
        if !self.model.is_open() && !self.surface.is_up() {
            return Ok(false);
        }
        self.model.set_light(kit.mode == ui::lucent::palette::Mode::Light);
        let view = self.model.view();
        self.surface.apply(kit, engine, &view, over_video, now, now_ns)
    }

    pub fn tick(&mut self, engine: &mut Engine, now: u64) -> bool {
        self.surface.tick(engine, now)
    }

    pub fn due_at(&self) -> Option<u64> {
        [ScreenModel::due_at(&self.model), self.surface.due_at()].into_iter().flatten().min()
    }

    pub fn restyle(&mut self) {
        self.surface.restyle();
        self.redraw = true;
    }

    /// The pages Home's search offers as answers.
    pub fn search_pages(&self) -> Vec<Value> {
        self.model.search_pages()
    }

    pub fn state(&self) -> Value {
        json!({"settings": self.model.debug()})
    }

    pub fn geometry(&self) -> Value {
        json!({"settings": self.surface.frames().iter().map(|(n, f)| json!({"name": n, "x": f.x, "y": f.y, "w": f.w, "h": f.h})).collect::<Vec<_>>()})
    }
}
