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
pub const HELPER_TOPICS: &str = "profiles,openapps,updateblock,panel,receiver,ai,ai.prompt,ha.link,plugins,screen,cameras,onhome,plex.account";

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

    pub fn take_theme_preview(&mut self) -> Option<Option<Value>> {
        self.model.take_theme_preview()
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
        self.model.set_text_scale(kit.text_scale);
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
    pub fn restyle_glass(&mut self, kit: &Kit) {
        self.surface.restyle_glass(kit);
        self.redraw = true;
    }


    /// The pages Home's search offers as answers.
    pub fn search_pages(&self) -> Vec<Value> {
        self.model.search_pages()
    }

    pub fn state(&self) -> Value {
        let mut s = self.model.debug();
        // the rows of the page that were drawn (a page that could not be drawn whole says so here)
        s["built"] = json!(self.surface.row_count_built());
        json!({"settings": s})
    }

    pub fn geometry(&self) -> Value {
        json!({"settings": self.surface.frames().iter().map(|(n, f)| json!({"name": n, "x": f.x, "y": f.y, "w": f.w, "h": f.h})).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Id;
    use crate::kit::lint::Lint;
    use crate::kit::testkit;
    use crate::settings::model::Target;
    use ui::lucent::palette::Mode;

    fn press(host: &mut SettingsHost, name: &str, t: u64) {
        host.update(&Input::Key { name: name.into(), down: true, repeat: false }, t);
        host.update(&Input::Key { name: name.into(), down: false, repeat: false }, t + 20);
    }
    #[test]
    fn glass_restyle_keeps_settings_controls_alive() {
        use compositor_protocol::Request;
        let (mut h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).unwrap();
        let theme = lumatv_tokens::Tokens::from_json(&json!({"glassStrength": {"clear": 0.8}, "wallpaperMotion": {"enabled": false}, "motion": {"transitions": true}})).json().clone();
        let mut host = SettingsHost::new(1000, &theme, &json!({}));
        host.open(Some("look"), 0);
        host.click(Target::Row { id: "glassstrength".into(), seg: None, value: None }, 0);
        host.apply(&mut kit, &mut engine, false, 1, 1_000_000).unwrap();
        h.commit();
        host.update(&Input::Key { name: "right".into(), down: true, repeat: false }, 2);
        let mut changed = theme;
        changed["glassStrength"]["clear"] = json!(0.81);
        let tokens = lumatv_tokens::Tokens::from_json(&changed);
        kit.retheme(tokens, Mode::Dark, 1.0);
        host.restyle_glass(&kit);
        host.apply(&mut kit, &mut engine, false, 2, 2_000_000).unwrap();
        let changes = h.commit();
        assert!(changes.iter().any(|r| matches!(r, Request::Restyle { .. })));
        assert!(changes.iter().any(|r| matches!(r, Request::Set { props, .. } if props.clip.is_some())), "the slider fill updates in place");
        assert!(!changes.iter().any(|r| matches!(r, Request::GroupDelete { .. })), "the settings rows must not be recreated");
        let page = host.model.view().page.unwrap();
        assert_eq!(page.rows.iter().find(|r| r.id == "glassstrength").unwrap().num, 81.0);
        assert!(page.rows.iter().any(|r| r.id == "wallpaper"));
        assert!(page.rows.iter().find(|r| r.id == "transitions").unwrap().on);
    }


    #[test]
    fn gallery_uses_renderers_effective_text_scale() {
        for (scale, columns) in [(1.0, 3), (2.0, 2)] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let mut engine = Engine::new(&kit).unwrap();
            let mut host = SettingsHost::new(1000, &json!({}), &json!({}));
            host.open(Some("themes"), 0);
            host.apply(&mut kit, &mut engine, false, 1, 1_000_000).unwrap();
            let page = host.model.view().page.unwrap();
            assert_eq!(page.rows.iter().find(|r| !r.cards.is_empty()).unwrap().cards.len(), columns);
        }
    }

    #[test]
    fn gallery_fits_loaded_home_budget_while_scrolling() {
        let (_h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).unwrap();
        let scene = kit.ui.scene.clone();
        let room = kit.room();
        let _home_groups: Vec<_> = (0..room.groups.saturating_sub(32)).map(|_| scene.group(None, compositor_protocol::GroupPose::IDENTITY).unwrap()).collect();
        let _home_layers: Vec<_> = (0..room.layers.saturating_sub(150)).map(|_| scene.shape(1, 1, 0, ui::components::plate::solid(compositor_protocol::Rgba(0, 0, 0, 255), 0.0), compositor_protocol::Props::at(0.0, 0.0))).collect();
        let mut host = SettingsHost::new(1000, &json!({}), &json!({}));
        host.open(Some("themes"), 1000);
        for i in 0..21u64 {
            let t = 2000 + i * 300;
            host.apply(&mut kit, &mut engine, false, t, t * 1_000_000).expect("gallery must fit");
            assert!(!scene.take_overflow(), "layer exhaustion at gallery step {i}");
            press(&mut host, if i % 3 == 2 { "down" } else { "right" }, t + 30);
        }
    }


    /// The pages and tabs this change added or changed (Hold gear, About this TV, Allow LG Remote Service, AV reset, Auto power sync and No
    /// signal image, Auto Tuning, LG logo), each walked row by row so the scrolled list is read at every place.
    #[test]
    fn the_pages_of_the_lg_menu_items_read_clean_at_both_text_sizes_in_both_looks() {
        let privacy = json!({"ok": true, "privacy": {"id": "privacy", "canAdmin": true, "items": [
            {"id": "remoteservice", "title": "Allow LG Remote Service", "detail": "LG's remote support.", "kind": "switch", "on": false, "available": true, "admin": true, "confirm": true},
            {"id": "consent.acrAllowed", "title": "Screen content recognition", "kind": "switch", "on": true, "available": true, "settable": true}]}});
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).unwrap();
                for (page, tab, label) in [
                    ("about", 0, "about:serial label"),
                    ("remote", 1, "gearhold label"),
                    ("privacy", 1, "privacy:remoteservice label"),
                    ("support", 4, "avreset label"),
                    ("inputs", 1, "general.noSignalScreenSaver label"),
                    ("channels", 0, "autotuning label"),
                    ("look", 3, "other.lgLogoDisplay label"),
                ] {
                    let mut host = SettingsHost::new(1000, &lumatv_tokens::Tokens::shipped().json().clone(), &json!({}));
                    host.model.docs.set("privacy", privacy.clone());
                    host.model.docs.set("factory", json!({"returnValue": true, "modelName": "WEBOS22.SKJY", "serialNumber": "403RMDV12345"}));
                    host.model.docs.set("os", json!({"returnValue": true, "core_os_release": "9.1.0-12345"}));
                    host.model.docs.set("firmware", json!({"returnValue": true, "configs": {"tv.nyx.firmwareVersion": "04.30.25"}}));
                    host.open(Some(page), 1000);
                    if tab > 0 {
                        host.click(Target::Tab(tab), 1100);
                    } else {
                        press(&mut host, "right", 1200);
                    }
                    let mut seen = Vec::new();
                    for i in 0..14u64 {
                        host.apply(&mut kit, &mut engine, false, 2300 + i * 100, (2300 + i * 100) * 1_000_000).expect("Settings is drawn");
                        let mut lint = Lint::new();
                        host.feed_lint(&mut kit, &mut lint);
                        let bad: Vec<String> = lint.findings().iter().map(|f| format!("{f:?}")).collect();
                        assert!(bad.is_empty(), "{page} tab {tab} {mode:?} at {scale}, step {i}: {}", bad.join("\n"));
                        seen.extend(lint.words().iter().map(|w| w.name.clone()));
                        press(&mut host, "down", 2350 + i * 100);
                    }
                    assert!(seen.iter().any(|n| n.starts_with(label)), "{page} tab {tab} {mode:?} at {scale} drew the row {label:?} among {seen:?}");
                }
                std::mem::forget(h);
            }
        }
    }

    /// A TV's Home over a full dock holds 64 of the scene's 96 groups. Settings opens in the 32 that are left and draws every row of the
    /// page: a plate for every row (two groups each) took 43 for the Inputs page, and the rows past the thirtieth group were never drawn.
    #[test]
    fn a_page_is_drawn_whole_in_the_groups_a_full_home_leaves_and_only_the_focus_has_a_plate() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let mut engine = Engine::new(&kit).unwrap();
            let scene = kit.ui.scene.clone();
            let _home: Vec<_> = (0..64).map(|_| scene.group(None, compositor_protocol::GroupPose::IDENTITY).expect("a group")).collect();
            let mut host = SettingsHost::new(1000, &lumatv_tokens::Tokens::shipped().json().clone(), &json!({}));
            host.open(Some("inputs"), 1000);
            let before = kit.room().groups;
            host.apply(&mut kit, &mut engine, false, 2000, 2_000_000_000).unwrap_or_else(|e| panic!("Settings is drawn in what Home leaves at {scale}: {e}"));
            assert!(!scene.take_overflow(), "no layer was refused at {scale}");
            let used = before - kit.room().groups;
            assert!(used <= 16, "{used} groups at {scale}");
            let built = host.state()["settings"]["built"].as_u64().unwrap();
            assert!(if scale == 1.0 { built == 7 } else { built >= 4 }, "{built} rows of the page are drawn at {scale}");
        }
    }

    /// The plate is made for the row the focus comes to, and the row the focus left keeps its own until its lift is at rest.
    #[test]
    fn the_focus_plate_is_made_for_the_row_it_comes_to_and_given_back_when_the_lift_is_at_rest() {
        let (_h, mut kit) = testkit::kit(1.0);
        let t = std::rc::Rc::new(std::cell::Cell::new(1_000_000_000u64));
        kit.ui.scene.set_clock({ let t = t.clone(); move || t.get() }, 0);
        let mut engine = Engine::new(&kit).unwrap();
        let mut host = SettingsHost::new(1000, &lumatv_tokens::Tokens::shipped().json().clone(), &json!({}));
        host.open(Some("inputs"), 1000);
        press(&mut host, "right", 1200);
        let at = |host: &mut SettingsHost, kit: &mut Kit, engine: &mut Engine, ms: u64| {
            t.set(ms * 1_000_000);
            host.apply(kit, engine, false, ms, ms * 1_000_000).expect("Settings is drawn");
        };
        let plates = |engine: &Engine| (0..7).map(|i| Id(10_000 + 128 * i)).chain((0..10).map(|i| Id(1_000 + 128 * i))).filter(|id| engine.flow.has(*id)).count();
        at(&mut host, &mut kit, &mut engine, 2000);
        let first = engine.focus.current().expect("the focus is on a row");
        assert!(engine.flow.has(first), "the row the focus is on has its plate");
        assert!(plates(&engine) <= 3, "{} plates", plates(&engine));
        press(&mut host, "down", 2100);
        at(&mut host, &mut kit, &mut engine, 2150);
        let second = engine.focus.current().expect("and moves down");
        assert_ne!(first, second);
        assert!(engine.flow.has(second) && engine.flow.has(first), "the new row has its plate and the old one keeps its own while it settles");
        at(&mut host, &mut kit, &mut engine, 3500);
        assert!(engine.flow.has(second) && !engine.flow.has(first), "the lift of the old row is at rest: its plate is given back");
        assert!(plates(&engine) <= 3, "{} plates", plates(&engine));
    }
}
