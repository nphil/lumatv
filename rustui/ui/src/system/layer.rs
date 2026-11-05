//! The system layer's kinds that sit over everything (the alerts, the overlay cards and the camera pop-up, the capture mark, the
//! screensaver) as one part of the shell: their models fed from the channel, the helper, the keys and the clock, their surfaces drawn with
//! the kit in the system band of the engine's z, and what the compositor is told of them (who has the keys, which keys the cards take, whether
//! anything is on the screen). Nothing here does I/O: effects go to the shell, which hands them to the host.
//!
//! The facts the kinds share are read here once, from the channel's topics: what is in front (an app, not Home, with the screen on), whether
//! the screen is on, whether the on-screen keyboard is up. A kind that must not disturb (the cards, the pop-up) asks [`Layer::busy`].

use super::ai::AiPrompt;
use super::ai_surface::AiSurface;
use super::banner::{self, Banner};
use super::banner_surface::BannerSurface;
use super::alert::{Alerts, Target};
use super::alert_surface::AlertSurface;
use super::capture::{Capture, CaptureSurface};
use super::extras::{Extras, View as ExtrasView};
use super::extras_surface::ExtrasSurface;
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
    pub const AI: i32 = 150;
    pub const BANNER: i32 = 35;
    pub const CAPTURE: i32 = 180;
}

/// The client name the saver votes on tvpower's lists with: the app id the QML vote used on the set (`SaverVote.qml`, `LS.appId`, the name LG's own
/// compositor registers under). A name of its own was never seen on the set's lists.
const SAVER_CLIENT: &str = "org.nphil.lumatv";

/// The tag the layout editor's write of `layout.overlays` comes back with.
pub const LAYOUT_TAG: u64 = 0x5359_0300;
/// The tag of the app launches the system layer asks for (a banner's buttons when the panels are not this process's).
pub const LAUNCH_TAG: u64 = 0x5359_0301;

/// What a card of the layer asks the shell to do, which is not the layer's to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// OK on the camera pop-up: the Cameras app's full view of this camera.
    OpenCamera(String),
    /// OK on a card with no action of its own: the notification drawer.
    OpenDrawer,
    /// Inputs on a banner: the input picker.
    OpenPicker,
    /// Picture on a banner: the quick panel.
    OpenQuick,
}

/// What a banner's buttons ask for, as the layer's intents (the input picker and the quick panel are other kinds' surfaces).
fn banner_intent(i: banner::Intent) -> Intent {
    match i {
        banner::Intent::OpenPicker => Intent::OpenPicker,
        banner::Intent::OpenQuick => Intent::OpenQuick,
    }
}

/// What a click on the extras landed on, as the model has it.
fn hit_of(hit: super::extras_surface::Hit) -> super::extras::Click {
    match hit {
        super::extras_surface::Hit::Pick(i) => super::extras::Click::Pick(i),
        super::extras_surface::Hit::Later => super::extras::Click::Later,
        super::extras_surface::Hit::Key(i) => super::extras::Click::Key(i),
    }
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
    /// The AI access prompt (kind `ai`): shown over everything, and no way to answer it ([`super::ai`]).
    ai: AiPrompt,
    ai_surface: AiSurface,
    /// The input banners (kind `banner`): the pill of an input that came to the front, the one with buttons, no signal, the channel banner.
    banner: Banner,
    banner_surface: BannerSurface,
    capture: Capture,
    capture_surface: CaptureSurface,
    /// What Home draws around itself (kind `home`): the launch screen, Quick Access, the on-screen remote, the voice bar, the mini frame.
    extras: Extras,
    extras_surface: ExtrasSurface,
    /// The view the extras' surface was last drawn from: a different one is a redraw.
    extras_seen: ExtrasView,
    /// The clock as the layer last heard it, for the timers of the extras' rings.
    now_ms: u64,
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
    interactive_visible: bool,
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
            extras: Extras::new(),
            extras_surface: ExtrasSurface::new(),
            extras_seen: ExtrasView::default(),
            now_ms: 0,
            overlay: Overlay::new(theme.get("overlays").unwrap_or(&Value::Null)),
            cards_surface: CardsSurface::new(base + z::CARDS, theme, layout),
            popup: Popup::new(),
            popup_surface: PopupSurface::new(base + z::POPUP, theme, layout),
            ai: AiPrompt::new(),
            ai_surface: AiSurface::new(base + z::AI),
            banner: Banner::new(theme),
            banner_surface: BannerSurface::new(base + z::BANNER),
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
            interactive_visible: false,
        }
    }

    /// An app, not Home, has the screen and the screen is on: what is drawn is over a picture (`Shell.qml`'s `appFront`).
    pub fn app_front(&self) -> bool {
        !self.foreground.home && !self.foreground.app.is_empty() && self.foreground.app != "com.webos.app.home" && self.screen_on
    }

    /// The app in front, when there is one (Home is none): what the Cameras full view takes the front from.
    pub fn foreground_app(&self) -> Option<String> {
        self.app_front().then(|| self.foreground.app.clone())
    }

    /// The camera card's picture, once, when OK on it opened the full view: the view's first picture.
    pub fn take_popup_picture(&mut self) -> Option<crate::kit::picture::Picture> {
        self.popup.take_expanded()
    }

    /// Something has the screen that the cards and the pop-up must wait for: an alert, the input popup, a PIN prompt, the keyboard, the
    /// screen off.
    pub fn busy(&self) -> bool {
        self.alerts.wants() || self.ai.wants() || self.banner.wants() || self.keyboard_up || !self.screen_on
    }

    /// Something that is not the layer's has the screen: a panel or Settings is up (`host_busy`), the Cameras app's viewer is up (`viewer_up`).
    /// The cards and the camera pop-up wait for it (the pop-up steps aside, the cards keep what they hold). The saver does not draw over the viewer or
    /// the pop-up either: they show a live picture.
    pub fn set_host(&mut self, kinds: &Kinds, host_busy: bool, viewer_up: bool, now: u64) -> Vec<Effect> {
        self.host_busy = host_busy;
        self.viewer_up = viewer_up;
        let mut out = if kinds.saver { self.saver.set_live_view(viewer_up || (kinds.popup && self.popup.up()), now) } else { Vec::new() };
        out.extend(self.sync_context(kinds, now));
        out
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

    /// What a banner's buttons asked for goes to the shell with the cards' intents.
    fn take_banner_intents(&mut self) {
        for i in self.banner.take_intents() {
            self.intents.push(banner_intent(i));
        }
    }

    /// Home told the extras something (an app opens, a digit did something, the remote key, a story opens).
    pub fn extras_note(&mut self, note: super::extras::Note, now: u64) -> Vec<Effect> {
        let out = self.extras.note(note, now);
        self.now_ms = now;
        self.redraw = true;
        out
    }

    /// A question of the system layer that is not Home's (an alert, the AI access prompt) is up: the extras of Home are not to be opened by any key
    /// while it is (the QML's `!aiPrompt.up`), and nothing the extras draw may answer it.
    pub fn questions_up(&self, kinds: &Kinds) -> bool {
        (kinds.alert && self.alerts.wants()) || (kinds.ai && self.ai.wants())
    }

    /// The voice key went down or up (the compositor sends it when the helper says voice search is ready).
    pub fn voice_key(&mut self, down: bool, now: u64) -> Vec<Effect> {
        self.now_ms = now;
        let out = self.extras.voice_key(down, now);
        self.redraw = true;
        out
    }

    /// Whether the voice key is the extras' to take (Home is in front and the helper can listen, or a request is under way).
    pub fn takes_voice(&self) -> bool {
        self.extras.takes_voice()
    }

    /// Whether a screen of what Home draws around itself is on the screen.
    pub fn extras_up(&self) -> bool {
        self.extras_surface.is_up()
    }

    /// What the extras asked Home for (a pick of the question, More).
    pub fn take_extras_intents(&mut self) -> Vec<super::extras::Intent> {
        self.extras.take_intents()
    }

    /// A banner asked for by name (`ctl shell show banner pill|bar|nosignal`, a TV check).
    pub fn show_banner(&mut self, page: &str, now: u64) -> Vec<Effect> {
        let out = self.banner.show_sample(page, now);
        self.redraw = true;
        out
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

    /// The claim the camera pop-up makes on a video window for its picture box, while it is up and the record says it can play live video.
    pub fn popup_claim(&self, kinds: &Kinds) -> Option<Value> {
        if kinds.popup { self.popup.claim() } else { None }
    }

    /// What the compositor made of that claim (the shell reads it from the `vwin` topic), and whether videooutputd lists the video on a sink.
    pub fn popup_window(&mut self, kinds: &Kinds, window: crate::cameras::app::Window, slot: &str, sink: bool, now: u64) -> Vec<Effect> {
        if kinds.popup { self.popup.set_window(window, slot, sink, now) } else { Vec::new() }
    }

    /// The subscriptions the kinds ask for at the start.
    pub fn start(&mut self, kinds: &Kinds) -> Vec<Effect> {
        self.popup.set_can_expand(kinds.cameras);
        self.overlay.set_drawer_ready(kinds.drawer);
        let mut out = Vec::new();
        if kinds.alert {
            out.extend(self.alerts.subscribe());
        }
        if kinds.home {
            out.extend(self.extras.start());
        }
        out
    }

    /// The owner's theme changed: the saver may have been turned off, given another brightness or its own time back.
    pub fn set_theme(&mut self, theme: &Value, now: u64) -> Vec<Effect> {
        self.capture.set_reduced(theme.pointer("/motion/reduced").and_then(Value::as_bool) == Some(true));
        self.overlay.set_config(theme.get("overlays").unwrap_or(&Value::Null));
        let mut out = self.banner.set_theme(theme, now);
        out.extend(self.saver.set_config(saver_config(theme), now));
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
        let alert_has_keys = (kinds.alert && self.alerts.wants()) || (kinds.ai && self.ai.wants());
        let saver_has_keys = kinds.saver && self.saver.wants_keys();
        if kinds.alert {
            out.extend(self.alerts.update(input, now));
        }
        if kinds.ai {
            out.extend(self.ai.update(input, now));
        }
        if kinds.banner {
            // its keys are its while it has buttons, unless a question of the alerts (or the editor) is the one that has them
            let key = matches!(input, Input::Key { .. });
            if !(key && (alert_has_keys || self.layout_open())) {
                out.extend(self.banner.update(input, now));
                self.take_banner_intents();
            }
        }
        if kinds.capture {
            out.extend(self.capture.update(input, now));
        }
        let key_like = matches!(input, Input::Key { .. } | Input::Pointer { .. });
        self.now_ms = now;
        // what Home draws around itself: its question, remote, frame and voice bar have the keys while they are up, unless a question of the alerts
        // (or the editor) has them
        if kinds.home && !(key_like && (alert_has_keys || self.layout_open() || saver_has_keys)) {
            out.extend(self.extras.update(input, now));
            let view = self.extras.view();
            if view != self.extras_seen {
                self.extras_seen = view;
                self.redraw = true;
            }
        }
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
        let key_theirs = !matches!(input, Input::Key { .. }) || !(alert_has_keys || saver_has_keys);
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
        let yes = !self.layout_open() && !(self.alerts.wants() && kinds.alert) && !(kinds.ai && self.ai.wants()) && !(kinds.saver && self.saver.wants_keys()) && ((kinds.popup && self.popup.armed(now)) || (kinds.cards && self.overlay.armed()));
        if yes {
            self.taken.push(name.clone());
        }
        yes
    }

    /// Whether the layout editor is up: it has every key and the pointer.
    pub fn layout_open(&self) -> bool {
        self.layout.is_some()
    }

    /// Whether the AI access prompt is up: it owns the keys and the pointer, and nothing under it hears either.
    pub fn ai_up(&self) -> bool {
        self.ai.up()
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
            video: false,
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

    pub(crate) fn interactive_visible(&self) -> bool { self.interactive_visible }

    pub(crate) fn screen_on(&self) -> bool { self.screen_on }

    /// Draws what changed. Returns whether anything of the layer is on the screen.
    pub fn apply(&mut self, kinds: &Kinds, kit: &mut Kit, engine: &mut Engine, now_ms: u64) -> Result<bool, String> {
        let now_ns = kit.ui.scene.now_ns();
        let mut live = false;
        let mut interactive = false;
        if kinds.alert {
            let view = self.alerts.view();
            match self.alert_surface.apply(kit, engine, &view, now_ms, now_ns) {
                Ok(on) => { live |= on; interactive |= on; },
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("alerts: {e}")); },
            }
        }
        if kinds.banner {
            let view = self.banner.view();
            match self.banner_surface.apply(kit, engine, view.as_ref(), self.app_front(), now_ms, now_ns) {
                Ok(on) => { live |= on; interactive |= on; },
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("banner: {e}")); },
            }
        }
        if kinds.ai {
            let view = self.ai.view(now_ms);
            match self.ai_surface.apply(kit, view.as_ref(), self.app_front(), now_ms) {
                Ok(on) => { live |= on; interactive |= on; },
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("ai prompt: {e}")); },
            }
        }
        if kinds.saver {
            let glance = self.glance(now_ms);
            match self.saver_surface.apply(kit, &mut self.saver, &glance, now_ms) {
                Ok(on) => live |= on,
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("saver: {e}")); },
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
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("camera pop-up: {e}")); },
            }
            // where the picture box is on the screen: what the live video's claim is for
            self.popup.set_picture_rect(self.popup_surface.picture_rect());
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
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("cards: {e}")); },
            }
        }
        if kinds.layout {
            if let Some(editor) = &self.layout {
                let view = editor.view(now_ms);
                match self.layout_surface.apply(kit, engine, &view, editor, now_ms, now_ns) {
                    Ok(on) => { live |= on; interactive |= on; },
                    Err(e) => { self.interactive_visible |= interactive; return Err(format!("layout editor: {e}")); },
                }
            } else if self.layout_surface.is_open() {
                self.layout_surface.close(engine);
            }
        }
        if kinds.home {
            self.extras.set_light(kit.mode == ui::lucent::palette::Mode::Light);
            let view = self.extras.view();
            let now_ns = kit.ui.scene.now_ns();
            match self.extras_surface.apply(kit, engine, &view, now_ms, now_ns) {
                Ok(on) => { live |= on; interactive |= on; },
                Err(e) => { self.interactive_visible |= interactive || self.extras_surface.interactive_visible(); return Err(format!("home extras: {e}")); },
            }
            if let Some(icon) = self.extras_surface.splash_icon() {
                self.extras.set_splash_icon(icon);
            }
            self.extras_seen = view;
        }
        if kinds.capture {
            match self.capture_surface.apply(kit, &self.capture.view(), over, now_ms) {
                Ok(on) => live |= on,
                Err(e) => { self.interactive_visible |= interactive; return Err(format!("capture: {e}")); },
            }
        }
        self.interactive_visible = interactive;
        Ok(live)
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.alert_surface.on_key(kit, engine);
        self.banner_surface.on_key(kit, engine);
        self.extras_surface.on_key(kit, engine);
        self.layout_surface.on_key(kit, engine);
    }

    /// A sample of the pointer: a click is the target to press.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, now_ms: u64, sample: Sample) -> Option<Target> {
        if self.banner.wants() && !self.alerts.wants() {
            // a banner with buttons: a click is the button it ended on, and what the pointer does here is not the screens under it
            if let Some(crate::system::banner_surface::Target::Button(i)) = self.banner_surface.pointer(kit, engine, now_ns, sample) {
                self.effects.extend(self.banner.picked(i));
                self.take_banner_intents();
                self.redraw = true;
            }
            return None;
        }
        if let Some(editor) = &mut self.layout {
            // the editor is the whole screen: the pointer is its own, and nothing under it sees it
            let effects = self.layout_surface.pointer(kit, engine, now_ns, now_ms, sample, editor);
            if !effects.is_empty() || self.layout_surface.take_dirty() {
                self.redraw = true;
            }
            self.editor_effects(effects);
            return None;
        }
        // what Home draws around itself that takes the pointer (the question, the on-screen remote): a click is what it ended on, and nothing under it hears the pointer
        if self.extras_surface.has_pointer() && !self.alerts.wants() {
            if let Some(hit) = self.extras_surface.pointer(kit, engine, now_ns, sample) {
                let mut out = Vec::new();
                self.extras.hit(hit_of(hit), &mut out);
                self.effects.extend(out);
                self.redraw = true;
            }
            return None;
        }
        self.alert_surface.pointer(kit, engine, now_ns, sample)
    }

    /// The layer's timers: the surfaces' fades are dropped when over, the saver's hands move. Returns whether a redraw is wanted.
    pub fn tick(&mut self, kit: &Kit, kinds: &Kinds, now_ms: u64) -> bool {
        let mut redraw = self.alert_surface.tick(now_ms);
        if kinds.saver {
            redraw |= self.saver_surface.tick(now_ms);
        }
        if kinds.banner {
            redraw |= self.banner_surface.tick(now_ms);
        }
        if kinds.ai {
            redraw |= self.ai_surface.tick(now_ms);
        }
        if kinds.capture {
            redraw |= self.capture_surface.tick(now_ms);
        }
        if kinds.home {
            redraw |= self.extras_surface.tick(kit, now_ms);
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
        if kinds.banner {
            due = [due, self.banner.due_at(), self.banner_surface.due_at()].into_iter().flatten().min();
        }
        if kinds.ai {
            due = [due, self.ai.due_at(), self.ai_surface.due_at()].into_iter().flatten().min();
        }
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
        if kinds.home {
            due = [due, self.extras.due_at(self.now_ms), self.extras_surface.due_at()].into_iter().flatten().min();
        }
        due
    }

    /// Whether the layer wants every key: an alert, the input popup or a PIN prompt is up, or the saver is (any key wakes it).
    pub fn wants(&self, kinds: &Kinds) -> bool {
        (kinds.alert && self.alerts.wants()) || (kinds.ai && self.ai.wants()) || (kinds.banner && self.banner.wants()) || (kinds.saver && self.saver.wants_keys()) || (kinds.layout && self.layout_open()) || (kinds.home && self.extras.wants())
    }

    /// The keys the layer takes by name while an app has the focus: the cards' OK and Back, and INFO (Back too while its banner is up) while the
    /// channel banner follows Live TV.
    pub fn takes(&self, kinds: &Kinds) -> Vec<&'static str> {
        let quiet = !(kinds.alert && self.alerts.wants()) && !(kinds.ai && self.ai.wants()) && !(kinds.banner && self.banner.wants()) && !(kinds.saver && self.saver.wants_keys());
        let mut keys = if quiet && !self.layout_open() && ((kinds.popup && self.popup.armed_now()) || (kinds.cards && self.overlay.armed())) { vec!["ok", "back"] } else { Vec::new() };
        if kinds.banner && quiet {
            for k in self.banner.takes() {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    }

    /// Whether the saver is up: what the compositor's OLED care and the helper's rules read (`state.saver`).
    pub fn saver_up(&self, kinds: &Kinds) -> bool {
        kinds.saver && self.saver.up()
    }

    /// The kinds' own words for the compositor: what the gate reads of `ctl shell`.
    pub fn state(&self, kinds: &Kinds) -> Value {
        let mut s = json!({});
        if kinds.banner {
            s["banner"] = self.banner.state();
        }
        if kinds.ai {
            s["ai"] = self.ai.state();
        }
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
        if kinds.home {
            s["extras"] = self.extras.state();
        }
        if kinds.cards {
            s["cards"] = self.overlay.state();
        }
        s
    }

    /// Where the surfaces are (the harness reads them).
    pub fn geometry(&self, kinds: &Kinds, kit: &Kit, engine: &Engine) -> Value {
        let mut g = json!({});
        if kinds.banner {
            g["banner"] = self.banner_surface.geometry();
        }
        if kinds.ai {
            g["ai"] = self.ai_surface.geometry();
        }
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
        if kinds.home {
            g["extras"] = self.extras_surface.geometry(engine);
        }
        if kinds.cards {
            g["cards"] = self.cards_surface.geometry();
        }
        g
    }

    /// Describes what is on the screen to a lint.
    pub fn feed_lint(&self, kinds: &Kinds, kit: &mut Kit, lint: &mut crate::kit::lint::Lint) {
        if kinds.banner {
            self.banner_surface.feed_lint(kit, lint);
        }
        if kinds.ai {
            self.ai_surface.feed_lint(kit, lint);
        }
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
        if kinds.home {
            self.extras_surface.feed_lint(kit, lint);
        }
    }

    /// The look changed: the surfaces are made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.alert_surface.restyle(engine);
        self.saver_surface.restyle();
        self.ai_surface.restyle();
        self.banner_surface.restyle(engine);
        self.capture_surface.restyle();
        self.popup_surface.restyle();
        self.cards_surface.restyle();
        self.layout_surface.restyle(engine);
        self.extras_surface.restyle(engine);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saver_wake_key_does_not_operate_home_extras_or_camera_popup() {
        let mut layer = Layer::new(0, &json!({}), &json!({}));
        let kinds = Kinds::parse("home,saver,popup,cards");
        layer.extras.note(super::super::extras::Note::Ask { digit: "1".into(), picks: vec![] }, 1000);
        layer.popup.set_can_expand(true);
        layer.popup.update(&Input::Helper { topic: "cameras.popup".into(), data: json!({"id": 8, "camera": "275", "kind": "person", "at": 1000, "untilMs": 30000}) }, 1000);
        assert!(layer.popup.armed(1700));
        layer.saver.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "game", "home": false}) }, 1700);
        layer.saver.update(&Input::Helper { topic: "screen".into(), data: json!({"on": true, "state": "Screen Saver"}) }, 1701);
        assert!(layer.saver.up());
        let ask = layer.extras.view().ask;
        let fx = layer.update(&kinds, &Input::Key { name: "ok".into(), down: true, repeat: false }, 1702);
        assert!(!layer.saver.up());
        assert_eq!(layer.extras.view().ask, ask);
        assert!(!fx.iter().any(|e| matches!(e, Effect::Post { path, .. } if path == "/v1/cameras/popup/clear")));
        assert!(!layer.take_intents().iter().any(|i| matches!(i, Intent::OpenCamera(_))));
    }

    #[test]
    fn the_saver_does_not_take_the_screen_from_the_cameras_viewer() {
        let mut layer = Layer::new(0, &json!({}), &json!({}));
        let kinds = Kinds::parse("home,saver,popup,cards,cameras");
        assert_eq!(layer.saver.state()["canDraw"], true);
        layer.set_host(&kinds, false, true, 1000);
        assert_eq!(layer.saver.state()["canDraw"], false, "the viewer shows a live picture");
        layer.set_host(&kinds, false, false, 1001);
        assert_eq!(layer.saver.state()["canDraw"], true);
        // a saver of a build without the kind has nothing to tell
        layer.set_host(&Kinds::parse("home"), false, true, 1002);
        assert_eq!(layer.saver.state()["canDraw"], true);
    }
}
