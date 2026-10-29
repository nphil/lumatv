//! The system layer's kinds that sit over everything (the alerts, the overlay cards and the camera pop-up, the capture mark, the
//! screensaver) as one part of the shell: their models fed from the channel, the helper, the keys and the clock, their surfaces drawn with
//! the kit in the system band of the engine's z, and what the compositor is told of them (who has the keys, which keys the cards take, whether
//! anything is on the screen). Nothing here does I/O: effects go to the shell, which hands them to the host.
//!
//! The facts the kinds share are read here once, from the channel's topics: what is in front (an app, not Home, with the screen on), whether
//! the screen is on, whether the on-screen keyboard is up. A kind that must not disturb (the cards, the pop-up) asks [`Layer::busy`].

use super::alert::{Alerts, Target};
use super::alert_surface::AlertSurface;
use super::plugins::Hub;
use super::saver::glance::{self, Glance};
use super::saver::look::Config as SaverConfig;
use super::saver::model::Saver;
use super::saver::surface::SaverSurface;
use crate::host::clock::{civil_at, system_offset_s};
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input, Model};
use crate::kit::{Engine, Kit};
use crate::shell::Kinds;
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
    /// What the plugin broker publishes (cards, notes, the saver topic): the saver's slots and notices, and the overlay queue's records.
    hub: Hub,
    /// The helper's `weather` topic as it last was.
    weather: Value,
    foreground: Foreground,
    screen_on: bool,
    keyboard_up: bool,
    /// Something the surfaces must draw again.
    redraw: bool,
}

impl Layer {
    /// `base` is the system band's first z; `theme` the merged theme and `layout` Home's layout, for the placements.
    pub fn new(base: i32, theme: &Value, layout: &Value) -> Self {
        let seed = crate::host::now_ms() ^ 0x5eed_5eed;
        Self {
            base,
            alerts: Alerts::new(),
            alert_surface: AlertSurface::new(base + z::ALERT, theme, layout),
            saver: Saver::new(saver_config(theme), SAVER_CLIENT, seed),
            saver_surface: SaverSurface::new(base + z::SAVER),
            hub: Hub::new(),
            weather: Value::Null,
            foreground: Foreground::default(),
            screen_on: true,
            keyboard_up: false,
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

    /// The subscriptions the kinds ask for at the start.
    pub fn start(&mut self, kinds: &Kinds) -> Vec<Effect> {
        let mut out = Vec::new();
        if kinds.alert {
            out.extend(self.alerts.subscribe());
        }
        out
    }

    /// The owner's theme changed: the saver may have been turned off, given another brightness or its own time back.
    pub fn set_theme(&mut self, theme: &Value, now: u64) -> Vec<Effect> {
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
            Input::Helper { topic, data } if topic == "weather" => {
                if *data != self.weather {
                    self.weather = data.clone();
                    self.redraw |= kinds.saver && self.saver.up();
                }
            }
            _ => {}
        }
        let changed = self.hub.update(input, now);
        if (changed.saver || changed.notes || changed.cards) && kinds.saver && self.saver.up() {
            self.redraw = true;
        }
        // an alert that has the keys takes them: the saver never sees a key that was meant to answer it
        let alert_has_keys = kinds.alert && self.alerts.wants();
        if kinds.alert {
            out.extend(self.alerts.update(input, now));
        }
        let key_like = matches!(input, Input::Key { .. } | Input::Pointer { .. });
        if kinds.saver && !(key_like && alert_has_keys) {
            out.extend(self.saver.update(input, now));
        }
        for e in &out {
            if matches!(e, Effect::Redraw) {
                self.redraw = true;
            }
        }
        out
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
        Ok(live)
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.alert_surface.on_key(kit, engine);
    }

    /// A sample of the pointer: a click is the target to press.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, sample: Sample) -> Option<Target> {
        self.alert_surface.pointer(kit, engine, now_ns, sample)
    }

    /// The layer's timers: the surfaces' fades are dropped when over, the saver's hands move. Returns whether a redraw is wanted.
    pub fn tick(&mut self, kit: &Kit, kinds: &Kinds, now_ms: u64) -> bool {
        let _ = kit;
        let mut redraw = self.alert_surface.tick(now_ms);
        if kinds.saver {
            redraw |= self.saver_surface.tick(now_ms);
        }
        redraw
    }

    pub fn due_at(&self, kinds: &Kinds) -> Option<u64> {
        let mut due = None;
        if kinds.alert {
            due = [due, self.alerts.due_at(), self.alert_surface.due_at()].into_iter().flatten().min();
        }
        if kinds.saver {
            due = [due, self.saver.due_at(), self.saver_surface.due_at(), self.hub.due_at()].into_iter().flatten().min();
        }
        due
    }

    /// Whether the layer wants every key: an alert, the input popup or a PIN prompt is up, or the saver is (any key wakes it).
    pub fn wants(&self, kinds: &Kinds) -> bool {
        (kinds.alert && self.alerts.wants()) || (kinds.saver && self.saver.wants_keys())
    }

    /// The keys the layer takes by name while an app has the focus (the cards' OK and Back).
    pub fn takes(&self, kinds: &Kinds) -> Vec<&'static str> {
        let _ = kinds;
        Vec::new()
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
        s
    }

    /// Where the surfaces are (the harness reads them).
    pub fn geometry(&self, kinds: &Kinds) -> Value {
        let mut g = json!({});
        if kinds.alert {
            g["alert"] = self.alert_surface.geometry();
        }
        if kinds.saver {
            g["saver"] = self.saver_surface.geometry();
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
    }

    /// The look changed: the surfaces are made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.alert_surface.restyle(engine);
        self.saver_surface.restyle();
        self.redraw = true;
    }

    /// The owner's places changed (`layout.overlays`).
    pub fn set_user(&mut self, user: lumatv_placement::User) {
        self.alert_surface.set_user(user);
        self.redraw = true;
    }

    pub fn base(&self) -> i32 {
        self.base
    }
}
