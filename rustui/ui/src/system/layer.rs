//! The system layer's kinds that sit over everything (the alerts, the overlay cards and the camera pop-up, the capture mark, the
//! screensaver) as one part of the shell: their models fed from the channel, the helper, the keys and the clock, their surfaces drawn with
//! the kit in the system band of the engine's z, and what the compositor is told of them (who has the keys, which keys the cards take, whether
//! anything is on the screen). Nothing here does I/O: effects go to the shell, which hands them to the host.
//!
//! The facts the kinds share are read here once, from the channel's topics: what is in front (an app, not Home, with the screen on), whether
//! the screen is on, whether the on-screen keyboard is up. A kind that must not disturb (the cards, the pop-up) asks [`Layer::busy`].

use super::alert::{Alerts, Target};
use super::alert_surface::AlertSurface;
use super::capture::{Capture, CaptureSurface};
use super::layout_surface::LayoutSurface;
use super::cards_surface::{CardsSurface, Column};
use super::overlay::{self, Overlay};
use super::plugins::Hub;
use super::popup::{self, Popup};
use super::popup_surface::PopupSurface;
use super::saver::glance::{self, Glance};
use super::saver::look::Config as SaverConfig;
use super::saver::model::Saver;
use super::saver::surface::SaverSurface;
use crate::host::clock::{civil_at, system_offset_s};
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input, Model};
use crate::kit::{Engine, Kit};
use crate::shell::Kinds;
use lumatv_placement::editor::{Editor, Key as EditorKey};
use lumatv_placement::{Effect as PlaceEffect, Frame as PlacementFrame, Registry, Sizes, User};
use serde_json::{Value, json};

/// Where each kind draws, as offsets from the system band's base (`system_z + kit::z::SYSTEM`): the volume display and the
/// toasts hold 0 to 19, the rest are here, the capture mark on top (as the QML's `CaptureMark` is).
pub mod z {
    pub const SAVER: i32 = 20;
    pub const CARDS: i32 = 60;
    pub const POPUP: i32 = 90;
    pub const ALERT: i32 = 110;
    pub const CAPTURE: i32 = 180;
}

/// The client name the saver votes on tvpower's lists with.
const SAVER_CLIENT: &str = "org.nphil.lumatv.saver";

/// The tag the layout editor's write of `layout.overlays` comes back with.
pub const LAYOUT_TAG: u64 = 0x5359_0300;

/// What a card of the layer asks the shell to do, which is not the layer's to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// OK on the camera pop-up: the Cameras app's full view of this camera.
    OpenCamera(String),
    /// OK on a card with no action of its own: the notification drawer.
    OpenDrawer,
}

/// What the channel says is in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foreground {
    pub app: String,
    pub home: bool,
}

/// The saver's settings out of the merged theme: the owner's `screensaver` block, the reduced-motion setting and the short fade it gets.
fn saver_config(theme: &Value) -> SaverConfig {
    let reduced = theme.pointer("/motion/reduced").and_then(Value::as_bool) == Some(true);
    let press = theme.pointer("/motion/pressMs").and_then(Value::as_u64).unwrap_or(90) as u32;
    SaverConfig::from_theme(theme, reduced, press)
}

pub struct Layer {
    base: i32,
    alerts: Alerts,
    alert_surface: AlertSurface,
    saver: Saver,
    saver_surface: SaverSurface,
    capture: Capture,
    capture_surface: CaptureSurface,
    overlay: Overlay,
    cards_surface: CardsSurface,
    popup: Popup,
    popup_surface: PopupSurface,
    /// The overlay layout editor (kind `layout`): the controller of the placement crate while it is open, and what draws it.
    layout: Option<Editor>,
    layout_surface: LayoutSurface,
    /// What the owner placed, and the frame Home gave: the editor starts from them.
    user: User,
    frame: PlacementFrame,
    /// Effects of the editor the shell carries (the write of the owner's places).
    effects: Vec<Effect>,
    /// What the plugin broker publishes (cards, notes, the saver topic): the saver's slots and notices, and the overlay queue's records.
    hub: Hub,
    /// The helper's `weather` topic as it last was.
    weather: Value,
    foreground: Foreground,
    screen_on: bool,
    keyboard_up: bool,
    /// Something of the host has the screen that the cards and the pop-up wait for (a panel, Settings), and the Cameras app's viewer is up.
    host_busy: bool,
    viewer_up: bool,
    /// The keys of OK and Back whose press a card took: their release and repeats are the card's too.
    taken: Vec<String>,
    intents: Vec<Intent>,
    /// Something the surfaces must draw again.
    redraw: bool,
}

impl Layer {
    /// `base` is the system band's first z; `theme` the merged theme and `layout` Home's layout, for the placements.
    pub fn new(base: i32, theme: &Value, layout: &Value) -> Self {
        // the seconds, not the milliseconds: under the harness's fixed clock the first spot is always the same one
        let seed = (crate::host::now_ms() / 1000) ^ 0x5eed_5eed;
        Self {
            base,
            alerts: Alerts::new(),
            alert_surface: AlertSurface::new(base + z::ALERT, theme, layout),
            saver: Saver::new(saver_config(theme), SAVER_CLIENT, seed),
            saver_surface: SaverSurface::new(base + z::SAVER),
            capture: Capture::new(theme.pointer("/motion/reduced").and_then(Value::as_bool) == Some(true)),
            capture_surface: CaptureSurface::new(base + z::CAPTURE),
            overlay: Overlay::new(theme.get("overlays").unwrap_or(&Value::Null)),
            cards_surface: CardsSurface::new(base + z::CARDS, theme, layout),
            popup: Popup::new(),
            popup_surface: PopupSurface::new(base + z::POPUP, theme, layout),
            layout: None,
            layout_surface: LayoutSurface::new(base + crate::kit::z::EDITOR - crate::kit::z::SYSTEM),
            user: Registry::shipped().user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null)),
            frame: PlacementFrame::from_json(layout, theme.pointer("/overlays/limit")),
            effects: Vec::new(),
            hub: Hub::new(),
            weather: Value::Null,
            foreground: Foreground::default(),
            screen_on: true,
            keyboard_up: false,
            host_busy: false,
            viewer_up: false,
            taken: Vec::new(),
            intents: Vec::new(),
            redraw: false,
        }
    }

    /// An app, not Home, has the screen and the screen is on: what is drawn is over a picture (`Shell.qml`'s `appFront`).
    pub fn app_front(&self) -> bool {
        !self.foreground.home && !self.foreground.app.is_empty() && self.foreground.app != "com.webos.app.home" && self.screen_on
    }

    /// Something has the screen that the cards and the pop-up must wait for: an alert, the input popup, a PIN prompt, the keyboard, the
    /// screen off.
    pub fn busy(&self) -> bool {
        self.alerts.wants() || self.keyboard_up || !self.screen_on
    }

    /// Something that is not the layer's has the screen: a panel or Settings is up (`host_busy`), the Cameras app's viewer is up (`viewer_up`).
    /// The cards and the camera pop-up wait for it (the pop-up steps aside, the cards keep what they hold).
    pub fn set_host(&mut self, kinds: &Kinds, host_busy: bool, viewer_up: bool, now: u64) -> Vec<Effect> {
        self.host_busy = host_busy;
        self.viewer_up = viewer_up;
        self.sync_context(kinds, now)
    }

    /// The facts the cards and the pop-up wait on, given to both.
    fn sync_context(&mut self, kinds: &Kinds, now: u64) -> Vec<Effect> {
        let busy = self.busy() || self.host_busy || self.layout_open() || (kinds.saver && self.saver.up());
        let mut out = Vec::new();
        if kinds.popup {
            out.extend(self.popup.set_context(busy, self.app_front(), self.viewer_up, now));
        }
        if kinds.cards {
            self.overlay.set_dnd(busy || self.viewer_up);
        }
        out
    }

    /// What the cards and the pop-up asked of the shell.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// The rectangles of the pop-up and the cards that are up: what the mute mark keeps clear of.
    pub fn avoid(&self, kinds: &Kinds) -> Vec<lumatv_placement::Rect> {
        let mut out = Vec::new();
        if kinds.popup {
            out.extend(self.popup_surface.frame());
        }
        if kinds.cards {
            out.extend(self.cards_surface.rect());
        }
        out
    }

    /// The subscriptions the kinds ask for at the start.
    pub fn start(&mut self, kinds: &Kinds) -> Vec<Effect> {
        self.popup.set_can_expand(kinds.cameras);
        self.overlay.set_drawer_ready(kinds.drawer);
        let mut out = Vec::new();
        if kinds.alert {
            out.extend(self.alerts.subscribe());
        }
        out
    }

    /// The owner's theme changed: the saver may have been turned off, given another brightness or its own time back.
    pub fn set_theme(&mut self, theme: &Value, now: u64) -> Vec<Effect> {
        self.capture.set_reduced(theme.pointer("/motion/reduced").and_then(Value::as_bool) == Some(true));
        self.overlay.set_config(theme.get("overlays").unwrap_or(&Value::Null));
        let out = self.saver.set_config(saver_config(theme), now);
        self.redraw = true;
        out
    }

    /// An input of the host to every kind of the layer.
    pub fn update(&mut self, kinds: &Kinds, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Topic { topic, data } => match topic.as_str() {
                "foreground" => {
                    self.foreground = Foreground {
                        app: data.get("app").and_then(Value::as_str).unwrap_or("").to_string(),
                        home: data.get("home").and_then(Value::as_bool).unwrap_or(false),
                    };
                }
                "screen" => self.screen_on = data.get("on").and_then(Value::as_bool) != Some(false),
                "keyboard" => self.keyboard_up = data.get("up").and_then(Value::as_bool) == Some(true),
                _ => {}
            },
            Input::Helper { topic, data } if topic == "weather" && *data != self.weather => {
                self.weather = data.clone();
                self.redraw |= kinds.saver && self.saver.up();
            }
            _ => {}
        }
        let changed = self.hub.update(input, now);
        if (changed.saver || changed.notes || changed.cards) && kinds.saver && self.saver.up() {
            self.redraw = true;
        }
        if kinds.cards {
            if !changed.posted.is_empty() {
                self.overlay.posted(&changed.posted, now);
            }
            if changed.notes {
                self.overlay.reconcile(&self.hub.notes());
            }
            if changed.cards {
                self.overlay.sync_cards(&self.hub.cards(), now);
            }
        }
        // an alert that has the keys takes them: the saver never sees a key that was meant to answer it
        let alert_has_keys = kinds.alert && self.alerts.wants();
        if kinds.alert {
            out.extend(self.alerts.update(input, now));
        }
        if kinds.capture {
            out.extend(self.capture.update(input, now));
        }
        let key_like = matches!(input, Input::Key { .. } | Input::Pointer { .. });
        if kinds.layout && self.layout_open() && !alert_has_keys {
            match input {
                Input::Key { name, down: true, .. } => {
                    if let Some(k) = EditorKey::parse(name)
                        && let Some(editor) = &mut self.layout
                    {
                        let effects = editor.key(k, now);
                        self.editor_effects(effects);
                    }
                    self.redraw = true;
                }
                // the Home key leaves it: what was in the hand is put back, what was put down is already written
                Input::Topic { topic, .. } if topic == "homekey" => {
                    self.layout = None;
                    self.redraw = true;
                }
                _ => {}
            }
        }
        if kinds.saver && !(key_like && alert_has_keys) {
            out.extend(self.saver.update(input, now));
        }
        // the pop-up and the cards: a key that answers an alert or wakes the saver was not meant for them
        let key_theirs = !matches!(input, Input::Key { .. }) || !(alert_has_keys || (kinds.saver && self.saver.wants_keys()));
        // the camera card is above the cards: a key it takes is not theirs
        let popup_had = kinds.popup && self.popup.armed(now);
        if kinds.popup && key_theirs {
            out.extend(self.popup.update(input, now));
        }
        if kinds.cards {
            if let Input::Key { name, down: true, repeat: false } = input
                && key_theirs
                && !popup_had
            {
                self.overlay.key(name, now, &mut out);
            }
            for i in self.overlay.take_intents() {
                match i {
                    overlay::Intent::OpenDrawer => self.intents.push(Intent::OpenDrawer),
                }
            }
        }
        if kinds.popup {
            for i in self.popup.take_intents() {
                match i {
                    popup::Intent::Expand(camera) => self.intents.push(Intent::OpenCamera(camera)),
                    popup::Intent::Shown(kind) => {
                        if kinds.cards {
                            self.overlay.camera_shown(&kind, now);
                        }
                    }
                }
            }
        }
        out.extend(self.sync_context(kinds, now));
        if kinds.cards {
            self.overlay.set_camera(kinds.popup && self.popup.up());
            if self.overlay.needs_refresh(now) {
                self.redraw = true;
            }
        }
        for e in &out {
            if matches!(e, Effect::Redraw) {
                self.redraw = true;
            }
        }
        out
    }

    /// Whether this key event is a card's: OK or Back while the top card (the camera pop-up first) has been up long enough for the keys to be its,
    /// and the release and the repeats of a press a card took. The shell does not hand such a key to the screens under the cards.
    pub fn takes_key(&mut self, kinds: &Kinds, input: &Input, now: u64) -> bool {
        let Input::Key { name, down, repeat } = input else { return false };
        if name != "ok" && name != "back" {
            return false;
        }
        if let Some(i) = self.taken.iter().position(|k| k == name) {
            if !*down {
                self.taken.remove(i);
            }
            return true;
        }
        if !*down || *repeat {
            return false;
        }
        let yes = !self.layout_open() && !(self.alerts.wants() && kinds.alert) && !(kinds.saver && self.saver.wants_keys()) && ((kinds.popup && self.popup.armed(now)) || (kinds.cards && self.overlay.armed()));
        if yes {
            self.taken.push(name.clone());
        }
        yes
    }

    /// Whether the layout editor is up: it has every key and the pointer.
    pub fn layout_open(&self) -> bool {
        self.layout.is_some()
    }

    /// What the editor asked for: the owner's places written whole, or to leave.
    fn editor_effects(&mut self, effects: Vec<PlaceEffect>) {
        for e in effects {
            match e {
                PlaceEffect::Save(map) => {
                    self.user = Registry::shipped().user_from_json(&map);
                    self.effects.push(Effect::Theme { tag: LAYOUT_TAG, patch: json!({"layout": {"overlays": map}}) });
                }
                PlaceEffect::Close => self.layout = None,
            }
        }
        self.redraw = true;
    }

    /// The effects the layer's surfaces asked for (the write of the owner's places).
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// What the stand-ins of the pop-up and the cards measure at the kit's text size (the others are the volume bar's, the toast's and the mark's).
    pub fn sample_sizes(&mut self, kit: &mut Kit, sizes: &mut Sizes) {
        let view = super::popup::View {
            up: true,
            camera: "1".into(),
            name: "Front door".into(),
            icon: "bell-ring",
            what: "Doorbell rang".into(),
            age: "Just now".into(),
            face: super::popup::Face::Live,
            can_expand: true,
            armed: true,
        };
        let (size, _) = self.popup_surface.measure(kit, &view);
        sizes.set("popup", size);
        let width = self.overlay.config().width as f32;
        let mut cfg_entry = super::cards::Config::from_theme(&json!({}));
        cfg_entry.enabled = true;
        let note = super::plugins::Note {
            plugin_id: "kitchen".into(),
            plugin: "Kitchen Show".into(),
            id: "p".into(),
            channel: "timer".into(),
            channel_title: "Kitchen Show".into(),
            channel_icon: String::new(),
            revision: 1,
            received_at: 0.0,
            expires_at: 0.0,
            title: "Pasta".into(),
            body: "Time\u{2019}s up".into(),
            actions: vec![json!({"id": "stop", "label": "Stop", "action": "stop"})],
            kind: "timer".into(),
            level: String::new(),
            privacy: String::new(),
            score: Value::Null,
            importance: String::new(),
            channel_kind: String::new(),
        };
        if let Some(entry) = super::cards::from_note(&note, &cfg_entry, 0) {
            sizes.set("cards", lumatv_placement::Size::new(width as i32, CardsSurface::height_of(kit, &entry, width, false).ceil() as i32));
        }
    }

    /// Hands the editor the sizes the shell measured and starts it (called by the shell with the whole set).
    pub fn start_layout(&mut self, item: Option<&str>, sizes: Sizes) {
        let mut editor = Editor::new(Registry::shipped().clone(), self.user.clone(), self.frame, sizes);
        editor.start(item);
        self.layout = Some(editor);
        self.redraw = true;
    }

    /// The text size changed while the editor is open: it draws its stand-ins at the sizes the shell measured again.
    pub fn set_layout_sizes(&mut self, sizes: Sizes) {
        if let Some(editor) = &mut self.layout {
            editor.set_sizes(sizes);
            self.redraw = true;
        }
    }

    /// A press of a surface (a click of the pointer).
    pub fn press(&mut self, target: Target) -> Vec<Effect> {
        let out = self.alerts.press(target);
        if out.iter().any(|e| matches!(e, Effect::Redraw)) {
            self.redraw = true;
        }
        out
    }

    /// Takes the flag a model or a surface raised that something is to be drawn again.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// What the saver says now: the date and, when the owner wants a glance, the weather, the slots and the notices.
    fn glance(&self, now_ms: u64) -> Glance {
        if !self.saver.up() {
            return Glance::default();
        }
        let civil = civil_at(now_ms, system_offset_s(now_ms));
        glance::build(&self.hub, &self.weather, &civil, now_ms, self.saver.config().glance)
    }

    /// Draws what changed. Returns whether anything of the layer is on the screen.
    pub fn apply(&mut self, kinds: &Kinds, kit: &mut Kit, engine: &mut Engine, now_ms: u64) -> Result<bool, String> {
        let now_ns = kit.ui.scene.now_ns();
        let mut live = false;
        if kinds.alert {
            let view = self.alerts.view();
            match self.alert_surface.apply(kit, engine, &view, now_ms, now_ns) {
                Ok(on) => live |= on,
                Err(e) => return Err(format!("alerts: {e}")),
            }
        }
        if kinds.saver {
            let glance = self.glance(now_ms);
            match self.saver_surface.apply(kit, &mut self.saver, &glance, now_ms) {
                Ok(on) => live |= on,
                Err(e) => return Err(format!("saver: {e}")),
            }
        }
        let over = self.app_front();
        if kinds.popup {
            let view = self.popup.view();
            if view.up {
                let (_, picture) = self.popup_surface.measure(kit, &view);
                self.popup.set_picture_size(picture.0, picture.1);
            }
            match self.popup_surface.apply(kit, &view, self.popup.best(), self.popup.revision, over, now_ms) {
                Ok(on) => live |= on,
                Err(e) => return Err(format!("camera pop-up: {e}")),
            }
        }
        if kinds.cards {
            let popup = if kinds.popup && self.popup.up() { self.popup_surface.frame().map(|r| lumatv_placement::Size::new(r.w, r.h)) } else { None };
            let (width, gap) = (self.overlay.config().width as f32, self.overlay.config().gap as f32);
            let drawer_ready = self.overlay.drawer_ready();
            if self.overlay.needs_refresh(now_ms) {
                let room = self.cards_surface.room(popup, width);
                self.overlay.refresh(room, &mut |e| f64::from(CardsSurface::height_of(kit, e, width, drawer_ready)), now_ms);
            }
            let column = Column { width, gap, popup, drawer_ready, over_video: over };
            match self.cards_surface.apply(kit, self.overlay.shown(), &column, now_ms) {
                Ok(on) => live |= on,
                Err(e) => return Err(format!("cards: {e}")),
            }
        }
        if kinds.layout {
            if let Some(editor) = &self.layout {
                let view = editor.view(now_ms);
                match self.layout_surface.apply(kit, engine, &view, editor, now_ms, now_ns) {
                    Ok(on) => live |= on,
                    Err(e) => return Err(format!("layout editor: {e}")),
                }
            } else if self.layout_surface.is_open() {
                self.layout_surface.close(engine);
            }
        }
        if kinds.capture {
            match self.capture_surface.apply(kit, &self.capture.view(), over, now_ms) {
                Ok(on) => live |= on,
                Err(e) => return Err(format!("capture: {e}")),
            }
        }
        Ok(live)
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.alert_surface.on_key(kit, engine);
        self.layout_surface.on_key(kit, engine);
    }

    /// A sample of the pointer: a click is the target to press.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, now_ms: u64, sample: Sample) -> Option<Target> {
        if let Some(editor) = &mut self.layout {
            // the editor is the whole screen: the pointer is its own, and nothing under it sees it
            let effects = self.layout_surface.pointer(kit, engine, now_ns, now_ms, sample, editor);
            if !effects.is_empty() || self.layout_surface.take_dirty() {
                self.redraw = true;
            }
            self.editor_effects(effects);
            return None;
        }
        self.alert_surface.pointer(kit, engine, now_ns, sample)
    }

    /// The layer's timers: the surfaces' fades are dropped when over, the saver's hands move. Returns whether a redraw is wanted.
    pub fn tick(&mut self, kit: &Kit, kinds: &Kinds, now_ms: u64) -> bool {
        let _ = kit;
        let mut redraw = self.alert_surface.tick(now_ms);
        if kinds.saver {
            redraw |= self.saver_surface.tick(now_ms);
        }
        if kinds.capture {
            redraw |= self.capture_surface.tick(now_ms);
        }
        if kinds.popup {
            redraw |= self.popup_surface.tick(now_ms);
        }
        if kinds.cards {
            redraw |= self.cards_surface.tick(now_ms) || self.overlay.needs_refresh(now_ms);
        }
        redraw
    }

    pub fn due_at(&self, kinds: &Kinds) -> Option<u64> {
        let mut due = None;
        if kinds.alert {
            due = [due, self.alerts.due_at(), self.alert_surface.due_at()].into_iter().flatten().min();
        }
        if kinds.saver {
            due = [due, self.saver.due_at(), self.saver_surface.due_at()].into_iter().flatten().min();
        }
        if kinds.saver || kinds.cards {
            due = [due, self.hub.due_at()].into_iter().flatten().min();
        }
        if kinds.popup {
            due = [due, self.popup.due_at(), self.popup_surface.due_at()].into_iter().flatten().min();
        }
        if kinds.cards {
            due = [due, self.overlay.due_at(), self.cards_surface.due_at()].into_iter().flatten().min();
        }
        if kinds.capture {
            due = [due, self.capture.due_at(), self.capture_surface.due_at()].into_iter().flatten().min();
        }
        due
    }

    /// Whether the layer wants every key: an alert, the input popup or a PIN prompt is up, or the saver is (any key wakes it).
    pub fn wants(&self, kinds: &Kinds) -> bool {
        (kinds.alert && self.alerts.wants()) || (kinds.saver && self.saver.wants_keys()) || (kinds.layout && self.layout_open())
    }

    /// The keys the layer takes by name while an app has the focus (the cards' OK and Back).
    pub fn takes(&self, kinds: &Kinds) -> Vec<&'static str> {
        let quiet = !(kinds.alert && self.alerts.wants()) && !(kinds.saver && self.saver.wants_keys());
        if quiet && !self.layout_open() && ((kinds.popup && self.popup.armed_now()) || (kinds.cards && self.overlay.armed())) { vec!["ok", "back"] } else { Vec::new() }
    }

    /// Whether the saver is up: what the compositor's OLED care and the helper's rules read (`state.saver`).
    pub fn saver_up(&self, kinds: &Kinds) -> bool {
        kinds.saver && self.saver.up()
    }

    /// The kinds' own words for the compositor: what the gate reads of `ctl shell`.
    pub fn state(&self, kinds: &Kinds) -> Value {
        let mut s = json!({});
        if kinds.alert {
            for (k, v) in self.alerts.state().as_object().into_iter().flatten() {
                s[k] = v.clone();
            }
        }
        if kinds.saver {
            s["saver"] = json!(self.saver.up());
            s["saverState"] = self.saver.state();
        }
        if kinds.capture {
            s["capture"] = self.capture.state(self.capture_surface.dot_up());
        }
        if kinds.popup {
            s["popup"] = self.popup.state();
        }
        if kinds.layout {
            s["layout"] = json!({"open": self.layout_open()});
        }
        if kinds.cards {
            s["cards"] = self.overlay.state();
        }
        s
    }

    /// Where the surfaces are (the harness reads them).
    pub fn geometry(&self, kinds: &Kinds, kit: &Kit) -> Value {
        let mut g = json!({});
        if kinds.alert {
            g["alert"] = self.alert_surface.geometry();
        }
        if kinds.saver {
            g["saver"] = self.saver_surface.geometry();
        }
        if kinds.capture {
            g["capture"] = self.capture_surface.geometry(kit);
        }
        if kinds.popup {
            g["popup"] = self.popup_surface.geometry();
        }
        if kinds.layout {
            g["layout"] = self.layout_surface.geometry();
        }
        if kinds.cards {
            g["cards"] = self.cards_surface.geometry();
        }
        g
    }

    /// Describes what is on the screen to a lint.
    pub fn feed_lint(&self, kinds: &Kinds, kit: &mut Kit, lint: &mut crate::kit::lint::Lint) {
        if kinds.alert {
            self.alert_surface.feed_lint(kit, lint);
        }
        if kinds.saver {
            self.saver_surface.feed_lint(lint);
        }
        if kinds.capture {
            self.capture_surface.feed_lint(kit, lint);
        }
        if kinds.popup {
            self.popup_surface.feed_lint(kit, lint);
        }
        if kinds.layout {
            self.layout_surface.feed_lint(kit, lint);
        }
        if kinds.cards {
            self.cards_surface.feed_lint(kit, lint);
        }
    }

    /// The look changed: the surfaces are made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.alert_surface.restyle(engine);
        self.saver_surface.restyle();
        self.capture_surface.restyle();
        self.popup_surface.restyle();
        self.cards_surface.restyle();
        self.layout_surface.restyle(engine);
        self.redraw = true;
    }

    /// The owner's places changed (`layout.overlays`).
    pub fn set_user(&mut self, user: lumatv_placement::User) {
        self.user = user.clone();
        self.popup_surface.set_user(user.clone());
        self.cards_surface.set_user(user.clone());
        self.alert_surface.set_user(user);
        self.redraw = true;
    }

    pub fn base(&self) -> i32 {
        self.base
    }
}
