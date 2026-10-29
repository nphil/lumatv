//! The system layer as the Rust shell answers it: the kinds the compositor's `--shell-rust` list names (`osd`: the volume and mute
//! display from the `audio` topic and the helper's `receiver` topic; `toast`: LG's toast feed), their models fed from the
//! channel, the helper, the keys and the clock, their surfaces drawn with the kit over the compositor's windows (stack 1, from
//! `system_z`), and the report the compositor reads (`POST /v1/ui`: `can`, `wants`, `live`, `state`). No I/O here: the host
//! (`main.rs`) carries out the [`Act`]s and feeds the events back, so the whole of it runs under a test with nothing but a clock.

use crate::cameras::app::Config as CamerasConfig;
use crate::cameras::host::Cameras;
use crate::home::{Home, HomeSurface};
use crate::host::http::{Event, Outbound};
use crate::host::keys::KeyEvent;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input, Model};
use crate::kit::{Engine, Kit};
use crate::system::alert::ALERT_TAG;
use crate::system::layer::Layer;
use crate::system::toast::{FEED_TAG, Toasts};
use crate::system::toast_surface::ToastSurface;
use crate::system::volume::Volume;
use crate::system::volume_surface::VolumeSurface;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// What the shell asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// A request on the UI channel.
    Channel(Outbound),
    /// A request on the helper's API.
    Helper(Outbound),
    Log(String),
}

/// The kinds this process may answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kinds {
    pub osd: bool,
    pub toast: bool,
    pub alert: bool,
    pub cards: bool,
    pub saver: bool,
    pub capture: bool,
    pub popup: bool,
    pub layout: bool,
    pub home: bool,
    pub cameras: bool,
}

impl Kinds {
    /// `osd,toast` as the compositor's `--shell-rust` names them; unknown names are ignored.
    pub fn parse(list: &str) -> Self {
        let mut k = Kinds::default();
        for name in list.split(',').map(str::trim) {
            match name {
                "osd" => k.osd = true,
                "toast" => k.toast = true,
                "alert" => k.alert = true,
                "cards" => k.cards = true,
                "saver" => k.saver = true,
                "capture" => k.capture = true,
                "popup" => k.popup = true,
                "layout" => k.layout = true,
                "home" => k.home = true,
                "cameras" => k.cameras = true,
                _ => {}
            }
        }
        k
    }

    pub fn names(&self) -> Vec<&'static str> {
        [
            (self.osd, "osd"),
            (self.toast, "toast"),
            (self.alert, "alert"),
            (self.cards, "cards"),
            (self.saver, "saver"),
            (self.capture, "capture"),
            (self.popup, "popup"),
            (self.layout, "layout"),
            (self.home, "home"),
            (self.cameras, "cameras"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }
}

pub struct Shell {
    pub kit: Kit,
    /// The focus engine every screen of the process shares: the registry of focusables (a modal kind opens a scope of its own) and the
    /// flowing lift that shows each move.
    pub engine: Engine,
    pub kinds: Kinds,
    volume: Volume,
    volume_surface: VolumeSurface,
    toasts: Toasts,
    toast_surface: ToastSurface,
    /// The system layer's kinds that sit over everything: the alerts, the cards, the pop-up, the capture mark, the saver.
    sys: Layer,
    home: Home,
    home_surface: HomeSurface,
    /// The Cameras app (the grid, the full view, the strip, the sheet, the options page): a kind of its own over the panels.
    cameras: Cameras,
    /// What the Cameras app asked of the shell and nobody has taken yet (the grid's `Settings` button).
    cameras_intents: Vec<crate::cameras::app::Intent>,
    /// The last report sent, so an unchanged one is not sent again.
    reported: Option<Value>,
    live: bool,
    redraw: bool,
    helper_up: bool,
    started: bool,
    /// The data directory (`theme/theme.json` is written under it) and the module's (`icons/`, `art/`).
    data_dir: Option<PathBuf>,
}

impl Shell {
    /// `icons` is the module's `icons/audio` directory; `theme` the merged theme and `layout` Home's layout, for the placements.
    pub fn new(kit: Kit, kinds: Kinds, system_z: i32, icons: &Path, theme: &Value, layout: &Value) -> std::io::Result<Self> {
        let engine = Engine::new(&kit).map_err(|e| std::io::Error::other(format!("{e}")))?;
        Ok(Self {
            volume: Volume::default(),
            volume_surface: VolumeSurface::new(system_z + crate::kit::z::SYSTEM, icons, theme, layout),
            toasts: Toasts::new(),
            toast_surface: ToastSurface::new(system_z + crate::kit::z::SYSTEM + 10, theme, layout),
            sys: Layer::new(system_z + crate::kit::z::SYSTEM, theme, layout),
            home: Home::new(theme.clone(), crate::home::grid::shipped(), layout.clone()),
            home_surface: HomeSurface::new(system_z),
            cameras: Cameras::new(system_z + crate::kit::z::CAMERAS, CamerasConfig::from_theme(&kit, theme)),
            cameras_intents: Vec::new(),
            kit,
            engine,
            kinds,
            reported: None,
            live: false,
            redraw: false,
            helper_up: false,
            started: false,
            data_dir: None,
        })
    }

    /// After a frame: the focusables that have come to rest give their lift track back (`now_ns` on the engine's clock).
    pub fn tidy(&mut self, now_ns: u64) {
        self.engine.flow.tidy(now_ns);
    }

    /// The module's directory (`<module>/icons/lucide` are the kit's icons) and the data directory (`theme/theme.json` is written
    /// there by [`Effect::Theme`]).
    pub fn dirs(&mut self, module: &Path, data: &Path) {
        self.kit.icons = crate::kit::icon::Icons::new(module);
        self.home_surface.dirs(module, data);
        self.data_dir = Some(data.to_path_buf());
    }

    /// What goes out at the start: the feeds this process subscribes to, and the first report.
    pub fn start(&mut self) -> Vec<Act> {
        self.started = true;
        let mut acts = Vec::new();
        if self.kinds.toast {
            for e in self.toasts.subscribe() {
                self.carry(e, crate::host::now_ms(), &mut acts);
            }
        }
        for e in self.sys.start(&self.kinds) {
            self.carry(e, crate::host::now_ms(), &mut acts);
        }
        if self.kinds.home {
            for e in self.home.start(crate::host::now_ms()) {
                self.carry(e, crate::host::now_ms(), &mut acts);
            }
        }
        acts.push(Act::Channel(Outbound { method: "POST", path: "/v1/ui".into(), body: Some(self.report()), tag: None }));
        acts
    }

    fn carry(&mut self, effect: Effect, now: u64, acts: &mut Vec<Act>) {
        match effect {
            Effect::Luna { tag, uri, payload, subscribe } => acts.push(Act::Channel(Outbound {
                method: "POST",
                path: "/v1/luna".into(),
                body: Some(json!({"tag": tag, "uri": uri, "payload": payload, "subscribe": subscribe})),
                tag: None,
            })),
            Effect::Cancel { tag } => {
                acts.push(Act::Channel(Outbound { method: "POST", path: "/v1/cancel".into(), body: Some(json!({"tag": tag})), tag: None }))
            }
            Effect::Post { path, body } => acts.push(Act::Helper(Outbound { method: "POST", path, body: Some(body), tag: None })),
            Effect::Fetch { tag, path } => acts.push(Act::Helper(Outbound { method: "GET", path, body: None, tag: Some(tag) })),
            Effect::Request { tag, method, path, body } => {
                let method = match method.as_str() {
                    "GET" => "GET",
                    "PUT" => "PUT",
                    "DELETE" => "DELETE",
                    _ => "POST",
                };
                acts.push(Act::Helper(Outbound { method, path, body, tag: Some(tag) }))
            }
            Effect::Channel { path, body } => acts.push(Act::Channel(Outbound { method: "POST", path, body: Some(body), tag: None })),
            Effect::Theme { tag, patch } => self.write_theme(tag, &patch, now, acts),
            Effect::Read { tag, path } => {
                let (status, data) = match self.data_dir.as_ref().map(|d| std::fs::read(d.join(&path))) {
                    Some(Ok(bytes)) => match serde_json::from_slice::<Value>(&bytes) {
                        Ok(v) => (200, v),
                        Err(e) => (500, json!({"error": e.to_string()})),
                    },
                    Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => (404, Value::Null),
                    Some(Err(e)) => (500, json!({"error": e.to_string()})),
                    None => (500, json!({"error": "no data directory"})),
                };
                self.feed(&Input::Fetched { tag, status, data }, now, acts);
            }
            Effect::Write { path, text } => {
                let written = self.data_dir.as_ref().ok_or_else(|| std::io::Error::other("no data directory")).and_then(|d| {
                    let full = d.join(&path);
                    if let Some(dir) = full.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    let tmp = full.with_extension("tmp");
                    std::fs::write(&tmp, text)?;
                    std::fs::rename(&tmp, &full)
                });
                if let Err(e) = written {
                    acts.push(Act::Log(format!("write {path}: {e}")));
                }
            }
            Effect::Log(line) => acts.push(Act::Log(line)),
            Effect::Redraw => self.redraw = true,
        }
    }

    /// Writes a theme patch under the data directory and tells the models how it went, with the merged file.
    fn write_theme(&mut self, tag: u64, patch: &Value, now: u64, acts: &mut Vec<Act>) {
        let (status, data) = match &self.data_dir {
            Some(dir) => match crate::host::theme::persist(dir, patch) {
                Ok(merged) => (200, merged),
                Err(e) => {
                    acts.push(Act::Log(format!("theme: {e}")));
                    (500, json!({"error": e.to_string()}))
                }
            },
            None => (500, json!({"error": "no data directory"})),
        };
        self.feed(&Input::Fetched { tag, status, data }, now, acts);
    }

    fn feed(&mut self, input: &Input, now: u64, acts: &mut Vec<Act>) {
        if self.kinds.osd {
            for e in self.volume.update(input, now) {
                self.carry(e, now, acts);
            }
        }
        if self.kinds.toast {
            for e in self.toasts.update(input, now) {
                self.carry(e, now, acts);
            }
        }
        for e in self.sys.update(&self.kinds, input, now) {
            self.carry(e, now, acts);
        }
        if self.kinds.home {
            // a system-layer kind that has the keys (a panel up over Home) takes them before Home sees them
            let taken = matches!(input, Input::Key { .. } | Input::Pointer { .. }) && self.system_wants();
            if !taken {
                for e in self.home.update(input, now) {
                    self.carry(e, now, acts);
                }
                for r in self.home.take_requests() {
                    // the Settings and the drawer are other kinds' surfaces: routed to them when they are in the list
                    acts.push(Act::Log(format!("home asks for {}", match r { crate::home::model::Request::Settings => "settings", crate::home::model::Request::Notifications => "notifications" })));
                }
            }
        }
        // the Cameras viewer takes the keys unless an alert, a PIN prompt or the keyboard is up over it
        if self.kinds.cameras && (!matches!(input, Input::Key { .. }) || !self.sys.busy()) {
            for e in self.cameras.update(input, now) {
                self.carry(e, now, acts);
            }
            self.cameras_intents.extend(self.cameras.take_intents());
        }
    }

    /// Whether a kind of the system layer has the keys now (an alert, the input popup, a PIN prompt; the panels when they land); the
    /// volume display and the toasts never do.
    pub fn system_wants(&self) -> bool {
        self.sys.wants(&self.kinds) || (self.kinds.cameras && self.cameras.wants())
    }

    /// An event of the UI channel.
    pub fn channel(&mut self, event: &Event, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        match event {
            Event::Up(_) => {}
            Event::State(topics) => {
                // A session starts from the state: every topic as if pushed, in the order the compositor keeps them (the audio
                // state read at the start opens nothing: the model knows a first report from a change).
                for (topic, data) in topics {
                    self.feed(&Input::Topic { topic: topic.clone(), data: data.clone() }, now, &mut acts);
                }
                // The report goes again: a compositor that restarted has forgotten it.
                self.reported = None;
            }
            Event::Topic { topic, data } if topic == "luna" => {
                if let (Some(tag), Some(payload)) = (data.get("tag").and_then(Value::as_u64), data.get("payload")) {
                    if tag == FEED_TAG {
                        acts.push(Act::Log("toast delivered to the client".into()));
                    } else if tag == ALERT_TAG {
                        acts.push(Act::Log("alert delivered to the client".into()));
                    }
                    self.feed(&Input::Luna { tag, payload: payload.clone() }, now, &mut acts);
                }
            }
            Event::Topic { topic, data } => {
                self.feed(&Input::Topic { topic: topic.clone(), data: data.clone() }, now, &mut acts);
            }
            Event::Answer { .. } | Event::Bytes { .. } => {}
        }
        self.after(now, &mut acts);
        acts
    }

    /// An event of the helper's API.
    pub fn helper(&mut self, event: &Event, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        match event {
            Event::Up(up) => {
                if *up != self.helper_up {
                    self.helper_up = *up;
                    self.feed(&Input::Helper { topic: "up".into(), data: json!(up) }, now, &mut acts);
                }
            }
            Event::State(topics) => {
                for (topic, data) in topics {
                    self.feed(&Input::Helper { topic: topic.clone(), data: data.clone() }, now, &mut acts);
                }
            }
            Event::Topic { topic, data } => self.feed(&Input::Helper { topic: topic.clone(), data: data.clone() }, now, &mut acts),
            // an answer goes to every model, each of which takes the tags it asked with and lets the rest go by
            Event::Answer { tag, status, body } => self.feed(&Input::Fetched { tag: *tag, status: *status, data: body.clone() }, now, &mut acts),
            Event::Bytes { tag, status, body } => self.feed(&Input::Bytes { tag: *tag, status: *status, bytes: body.clone() }, now, &mut acts),
        }
        self.after(now, &mut acts);
        acts
    }

    /// A key or a pointer sample from the key socket: the layer's alerts take the keys while one is up, the pointer's hover and click
    /// are the kit's rule.
    pub fn key(&mut self, event: &KeyEvent, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        match event {
            KeyEvent::Key { name, down, repeat, .. } => {
                if *down {
                    self.sys.on_key(&self.kit, &mut self.engine);
                    if self.kinds.cameras && self.cameras.wants() {
                        self.cameras.surface.on_key(&mut self.kit, &mut self.engine);
                    }
                }
                self.feed(&Input::Key { name: name.clone(), down: *down, repeat: *repeat }, now, &mut acts);
            }
            KeyEvent::Pointer { x, y, buttons, visible } => {
                let now_ns = self.kit.ui.scene.now_ns();
                if let Some(target) = self.sys.pointer(&self.kit, &mut self.engine, now_ns, Sample { x: *x, y: *y, buttons: *buttons, visible: *visible }) {
                    for e in self.sys.press(target) {
                        self.carry(e, now, &mut acts);
                    }
                }
                if self.kinds.cameras {
                    let sample = Sample { x: *x, y: *y, buttons: *buttons, visible: *visible };
                    for e in self.cameras.pointer(&mut self.kit, &mut self.engine, now_ns, now, sample) {
                        self.carry(e, now, &mut acts);
                    }
                    self.cameras_intents.extend(self.cameras.take_intents());
                }
                self.feed(&Input::Pointer { x: *x, y: *y, buttons: *buttons, visible: *visible }, now, &mut acts);
            }
            KeyEvent::Closed => {}
        }
        self.after(now, &mut acts);
        acts
    }

    /// Time passed: the models' timers and the surfaces' own.
    pub fn tick(&mut self, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        self.feed(&Input::Tick, now, &mut acts);
        if self.volume_surface.tick(&mut self.kit, now) {
            self.redraw = true;
        }
        if self.sys.tick(&self.kit, &self.kinds, now) {
            self.redraw = true;
        }
        if self.kinds.home && self.home_surface.tick(now) {
            self.redraw = true;
        }
        if self.kinds.cameras && self.cameras.surface.tick(now) {
            self.redraw = true;
        }
        self.after(now, &mut acts);
        acts
    }

    /// Draws what changed and reports when the report changed.
    fn after(&mut self, now: u64, acts: &mut Vec<Act>) {
        if self.sys.take_redraw() {
            self.redraw = true;
        }
        if self.redraw {
            self.redraw = false;
            let mut live = false;
            if self.kinds.osd {
                let view = self.volume.view();
                match self.volume_surface.apply(&mut self.kit, &view, now) {
                    Ok(on) => live |= on,
                    Err(e) => acts.push(Act::Log(format!("volume: {e}"))),
                }
            }
            if self.kinds.toast {
                let view = self.toasts.view();
                match self.toast_surface.apply(&mut self.kit, &view) {
                    Ok(on) => live |= on,
                    Err(e) => acts.push(Act::Log(format!("toasts: {e}"))),
                }
            }
            match self.sys.apply(&self.kinds, &mut self.kit, &mut self.engine, now) {
                Ok(on) => live |= on,
                Err(e) => acts.push(Act::Log(e)),
            }
            if self.kinds.home {
                match self.home_surface.apply(&mut self.kit, &mut self.engine, &mut self.home, now) {
                    Ok(on) => live |= on,
                    Err(e) => acts.push(Act::Log(format!("home: {e}"))),
                }
            }
            if self.kinds.cameras {
                match self.cameras.apply(&mut self.kit, &mut self.engine, now) {
                    Ok(on) => live |= on,
                    Err(e) => acts.push(Act::Log(format!("cameras: {e}"))),
                }
            }
            self.live = live;
        }
        if self.started {
            let report = self.report();
            if self.reported.as_ref() != Some(&report) {
                self.reported = Some(report.clone());
                acts.push(Act::Channel(Outbound { method: "POST", path: "/v1/ui".into(), body: Some(report), tag: None }));
            }
        }
    }

    /// The report the compositor reads: what this process can show (the layout editor, when it is a kind), whether its layer wants
    /// the keys (an alert, the input popup or a PIN prompt is up), the keys its cards take while an app has the focus, whether it has
    /// something on screen, and its state for the gate.
    pub fn report(&self) -> Value {
        let mut can: Vec<&str> = Vec::new();
        if self.kinds.layout {
            can.push("layout");
        }
        if self.kinds.cameras {
            can.push("cameras");
        }
        // Home in front wants the keys too: the compositor sends every key of the remote to the layer that wants them
        let wants = self.sys.wants(&self.kinds) || (self.kinds.home && self.home.active) || (self.kinds.cameras && self.cameras.wants());
        json!({"can": can, "wants": wants, "takes": self.sys.takes(&self.kinds), "live": self.live, "state": self.state()})
    }

    pub fn state(&self) -> Value {
        let mut s = json!({"kinds": self.kinds.names(), "osd": false, "toasts": 0});
        if self.kinds.osd {
            let v = self.volume.state();
            for (k, val) in v.as_object().into_iter().flatten() {
                s[k] = val.clone();
            }
        }
        if self.kinds.toast {
            s["toasts"] = self.toasts.state();
        }
        for (k, val) in self.sys.state(&self.kinds).as_object().into_iter().flatten() {
            s[k] = val.clone();
        }
        if self.kinds.home {
            s["home"] = self.home.state();
        }
        if self.kinds.cameras {
            s["cameras"] = self.cameras.state();
        }
        s["live"] = json!(self.live);
        s
    }

    /// When the shell next wants a tick (the models' timers, a hop's pause).
    pub fn due_at(&self) -> Option<u64> {
        let mut due = None;
        if self.kinds.osd {
            due = [due, self.volume.due_at(), self.volume_surface.due_at()].into_iter().flatten().min();
        }
        if self.kinds.toast {
            due = [due, self.toasts.due_at()].into_iter().flatten().min();
        }
        if self.kinds.home {
            due = [due, self.home.due_at(), self.home_surface.due_at()].into_iter().flatten().min();
        }
        let cameras = if self.kinds.cameras { self.cameras.due_at() } else { None };
        [due, self.sys.due_at(&self.kinds), cameras].into_iter().flatten().min()
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    /// The volume pane's rectangle while it is up, and the toasts' cards (the host harness reads them).
    pub fn geometry(&self) -> Value {
        let mut g = json!({
            "osd": self.volume_surface.pane_rect().map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})),
            "toasts": self.toast_surface.rects().iter().map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})).collect::<Vec<_>>(),
        });
        for (k, v) in self.sys.geometry(&self.kinds).as_object().into_iter().flatten() {
            g[k] = v.clone();
        }
        if self.kinds.home {
            g["home"] = self.home_surface.geometry(&self.home);
        }
        if self.kinds.cameras {
            g["cameras"] = self.cameras.geometry();
        }
        g
    }

    /// The layout and contrast lint of what the layer has on the screen (`--eval lint`): an object per finding.
    pub fn lint(&mut self) -> Value {
        let mut lint = crate::kit::lint::Lint::new();
        self.sys.feed_lint(&self.kinds, &mut self.kit, &mut lint);
        if self.kinds.home {
            lint = self.home_surface.lint(&mut self.kit, &self.home);
        }
        if self.kinds.cameras {
            self.cameras.surface.feed_lint(&mut self.kit, &mut lint);
        }
        json!(lint.findings().iter().map(|f| json!({"kind": f.kind.name(), "subject": f.subject, "detail": f.detail})).collect::<Vec<_>>())
    }

    /// The volume model's own account (`--eval volume`).
    pub fn debug(&self) -> Value {
        self.volume.debug()
    }

    /// The theme or the text size changed: the surfaces are made again at the next draw.
    pub fn restyle(&mut self) {
        self.volume_surface.restyle();
        self.toast_surface.restyle();
        self.sys.restyle(&mut self.engine);
        self.home_surface.restyle();
        self.cameras.restyle(&mut self.engine, self.kit.text_scale);
        self.redraw = true;
    }

    /// The Home model, for the harness's `--eval` and the tests.
    pub fn home(&self) -> &Home {
        &self.home
    }

    /// What the Cameras app asked of the shell since the last call: Settings, Home & look, Cameras.
    pub fn take_cameras_intents(&mut self) -> Vec<crate::cameras::app::Intent> {
        std::mem::take(&mut self.cameras_intents)
    }

    /// The Cameras app, for the screens that open it (the pop-up's OK, the quick panel's tile).
    pub fn cameras(&mut self) -> &mut Cameras {
        &mut self.cameras
    }
}
