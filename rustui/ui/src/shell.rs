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
use crate::settings::host::SettingsHost;
use crate::settings::row::ShellRequest;
use crate::system::alert::ALERT_TAG;
use crate::panels::Typed;
use crate::panels::layer::{Panels, Which};
use crate::system::layer::Layer;
use crate::system::toast::{FEED_TAG, Toasts};
use crate::system::toast_surface::ToastSurface;
use crate::system::volume::Volume;
use crate::system::volume_surface::VolumeSurface;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The Settings page the Cameras grid's `Settings` button opens (a page of Home & look).
const CAMERAS_SETTINGS_PAGE: &str = "cameras";

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
    pub settings: bool,
    pub quick: bool,
    pub picker: bool,
    pub drawer: bool,
    pub menu: bool,
    pub confirm: bool,
    pub switcher: bool,
    pub entry: bool,
    /// The AI access prompt: the question the helper asks on the TV, shown over everything with no way to answer it.
    pub ai: bool,
    /// The input banners: the pill of an input that came to the front, the one with buttons, the no-signal screen and the channel banner.
    pub banner: bool,
    pub keyboard: bool,
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
                "settings" => k.settings = true,
                "quick" => k.quick = true,
                "picker" => k.picker = true,
                "drawer" => k.drawer = true,
                "menu" => k.menu = true,
                "confirm" => k.confirm = true,
                "switcher" => k.switcher = true,
                "entry" => k.entry = true,
                "ai" => k.ai = true,
                "banner" => k.banner = true,
                "keyboard" => k.keyboard = true,
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
            (self.settings, "settings"),
            (self.quick, "quick"),
            (self.picker, "picker"),
            (self.drawer, "drawer"),
            (self.menu, "menu"),
            (self.confirm, "confirm"),
            (self.switcher, "switcher"),
            (self.entry, "entry"),
            (self.ai, "ai"),
            (self.banner, "banner"),
            (self.keyboard, "keyboard"),
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
    /// The text of the layout file Settings has seen (Home's), to hand it the layout again when Home changes it.
    settings_layout: String,
    home_surface: HomeSurface,
    /// The Cameras app (the grid, the full view, the strip, the sheet, the options page): a kind of its own over the panels.
    cameras: Cameras,
    /// The display's spare video windows the screens' live pictures are asked for (`--vwin` of the compositor): the claims posted and what the topic answered.
    vwin: crate::host::vwin::Vwin,
    /// Settings, the modal kind over everything but the system layer.
    settings: SettingsHost,
    /// The panels: the quick panel, the input picker and the notification drawer.
    panels: Panels,
    /// The press of OK or Back that is being routed was taken by a card (the pop-up's or the overlay queue's): the screens under do not see it.
    cards_took: bool,
    /// The last report sent, so an unchanged one is not sent again.
    reported: Option<Value>,
    live: bool,
    redraw: bool,
    helper_up: bool,
    started: bool,
    /// The data directory (`theme/theme.json` is written under it) and the module's (`icons/`, `art/`).
    data_dir: Option<PathBuf>,
    /// The keys whose press the entry sheet took (their release is the sheet's too).
    entry_keys: Vec<String>,
}

impl Shell {
    /// `icons` is the module's `icons/audio` directory; `theme` the merged theme and `layout` Home's layout, for the placements.
    pub fn new(kit: Kit, kinds: Kinds, system_z: i32, icons: &Path, theme: &Value, layout: &Value) -> std::io::Result<Self> {
        let engine = Engine::new(&kit).map_err(|e| std::io::Error::other(format!("{e}")))?;
        let home = Home::new(theme.clone(), crate::home::grid::shipped(), layout.clone());
        Ok(Self {
            volume: Volume::default(),
            volume_surface: VolumeSurface::new(system_z + crate::kit::z::SYSTEM, icons, theme, layout),
            toasts: Toasts::new(),
            toast_surface: ToastSurface::new(system_z + crate::kit::z::SYSTEM + 10, theme, layout),
            sys: Layer::new(system_z + crate::kit::z::SYSTEM, theme, layout),
            // Settings reads the layout as Home has merged and tidied it
            settings: SettingsHost::new(system_z, theme, &home.layout),
            settings_layout: home.layout_text.clone(),
            home,
            home_surface: HomeSurface::new(system_z),
            cameras: Cameras::new(system_z + crate::kit::z::CAMERAS, CamerasConfig::from_theme(&kit, theme)),
            vwin: crate::host::vwin::Vwin::new(),
            panels: Panels::new(system_z + crate::kit::z::PANELS, theme),
            kit,
            engine,
            kinds,
            cards_took: false,
            reported: None,
            live: false,
            redraw: false,
            helper_up: false,
            started: false,
            data_dir: None,
            entry_keys: Vec::new(),
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
        for e in self.panels.start(&self.kinds) {
            self.carry(e, crate::host::now_ms(), &mut acts);
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
                let (status, data) = match self.file_of(&path).map(std::fs::read) {
                    Ok(Ok(bytes)) => match serde_json::from_slice::<Value>(&bytes) {
                        Ok(v) => (200, v),
                        Err(e) => (500, json!({"error": e.to_string()})),
                    },
                    Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => (404, Value::Null),
                    Ok(Err(e)) => (500, json!({"error": e.to_string()})),
                    Err(e) => (500, json!({"error": e.to_string()})),
                };
                self.feed(&Input::Fetched { tag, status, data }, now, acts);
            }
            Effect::Write { path, text } => {
                let written = self.file_of(&path).and_then(|full| {
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

    /// The file a model's effect names: a path under the data directory, or a file of the video-plane player's mailbox (`/tmp/<shortId>-vplane/`),
    /// which is the only place outside the data directory a model may read or write.
    fn file_of(&self, path: &str) -> std::io::Result<PathBuf> {
        if let Some(name) = path.strip_prefix(crate::home::wallvideo::MAILBOX).and_then(|rest| rest.strip_prefix('/')) {
            return if !name.is_empty() && !name.contains('/') && !name.contains("..") { Ok(PathBuf::from(path)) } else { Err(std::io::Error::other("not a file of the player's mailbox")) };
        }
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(std::io::Error::other("a path outside the data directory"));
        }
        self.data_dir.as_ref().map(|d| d.join(path)).ok_or_else(|| std::io::Error::other("no data directory"))
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
        // the owner's places the editor wrote: every surface reads them from now on, and Settings' rows say them in words
        if tag == crate::system::layer::LAYOUT_TAG && status == 200 {
            self.set_places(&data);
            self.settings.set_theme(data.clone());
        }
        self.feed(&Input::Fetched { tag, status, data }, now, acts);
    }

    fn feed(&mut self, input: &Input, now: u64, acts: &mut Vec<Act>) {
        // Who had the keys when this input arrived: a key that answers an alert or wakes the saver was theirs, and the screens under them
        // never see it (the key that wakes the TV does nothing else).
        let key_like = matches!(input, Input::Key { .. } | Input::Pointer { .. });
        let cards_had = matches!(input, Input::Key { .. }) && self.cards_took;
        let layer_had = key_like && (self.sys.wants(&self.kinds) || cards_had);
        let system_had = key_like && (self.system_wants() || cards_had);
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
        for e in self.panels.update(&self.kinds, input, now) {
            self.carry(e, now, acts);
        }
        self.panel_events(now, acts);
        if self.kinds.home {
            // a system-layer kind that has the keys (a panel up over Home) takes them before Home sees them
            let taken = system_had;
            if !taken {
                for e in self.home.update(input, now) {
                    self.carry(e, now, acts);
                }
                for r in self.home.take_requests() {
                    // the Settings and the drawer are other kinds' surfaces: routed to them when they are in the list
                    match r {
                        crate::home::model::Request::Settings if self.kinds.settings => {
                            for e in self.settings.open(None, now) {
                                self.carry(e, now, acts);
                            }
                            self.redraw = true;
                        }
                        // the bell opens the drawer when it is in the list
                        crate::home::model::Request::Notifications if self.kinds.drawer => {
                            let mut out = Vec::new();
                            self.panels.show(Which::Drawer, false, &self.kinds, now, &mut out);
                            for e in out {
                                self.carry(e, now, acts);
                            }
                        }
                        // the app switcher: the second press of Home with nothing open on Home, and Back from the top of Home when the owner wants it
                        crate::home::model::Request::Switcher => {
                            let wanted = self.panels.recents_on() || matches!(input, Input::Topic { topic, .. } if topic == "homekey");
                            if wanted && let Some(e) = self.panels.open_switcher(&self.kinds) {
                                self.carry(e, now, acts);
                            }
                        }
                        // Settings at a page: Quick Access's More, a Search answer that is a page
                        crate::home::model::Request::SettingsPage(page) if self.kinds.settings => {
                            for e in self.settings.open(Some(&page), now) {
                                self.carry(e, now, acts);
                            }
                            self.redraw = true;
                        }
                        crate::home::model::Request::SettingsPage(page) => acts.push(Act::Log(format!("home asks for settings at {page}"))),
                        crate::home::model::Request::Settings => acts.push(Act::Log("home asks for settings".into())),
                        crate::home::model::Request::Notifications => acts.push(Act::Log("home asks for notifications".into())),
                    }
                }
            }
        }
        self.extras_flow(now, acts);
        // Search has just opened: its answers may be pages of Settings, which only this process's Settings kind knows
        if self.kinds.home && self.kinds.settings && self.home.take_pages_wanted() {
            self.home.set_search_pages(self.settings.search_pages());
            self.redraw = true;
        }
        // the Cameras viewer takes the keys unless an alert, a PIN prompt or the keyboard is up over it, or Settings, which its grid opens
        let over = matches!(input, Input::Key { .. }) && (self.sys.busy() || (self.kinds.settings && self.settings.is_open()));
        if self.kinds.cameras && !over && !layer_had {
            for e in self.cameras.update(input, now) {
                self.carry(e, now, acts);
            }
            self.cameras_asks(now, acts);
        }
        if self.kinds.settings && !layer_had {
            for e in self.settings.update(input, now) {
                self.carry(e, now, acts);
            }
        }
        // what the screens want on a video window now, and the answers of the compositor's `vwin` topic
        let mut fx = self.vwin.update(input, now);
        fx.extend(self.vwin.want(&self.vwin_wants(), now));
        for e in fx {
            self.carry(e, now, acts);
        }
    }

    /// What Home draws around itself (the launch screen, Quick Access, the remote, the voice bar, the mini frame) and Home tell each other: the
    /// question's answer goes to Home, and what Home did goes to the screens.
    fn extras_flow(&mut self, now: u64, acts: &mut Vec<Act>) {
        if !self.kinds.home {
            return;
        }
        let mut out = Vec::new();
        for intent in self.sys.take_extras_intents() {
            self.home.extras_intent(intent, &mut out);
        }
        for note in self.home.take_notes() {
            out.extend(self.sys.extras_note(note, now));
        }
        for e in out {
            self.carry(e, now, acts);
        }
    }

    /// Whether a kind of the system layer has the keys now (an alert, the input popup, a PIN prompt; the panels when they land); the
    /// volume display and the toasts never do.
    pub fn system_wants(&self) -> bool {
        self.sys.wants(&self.kinds) || self.panels.wants(&self.kinds) || (self.kinds.cameras && self.cameras.wants())
    }

    /// What the panels tell the rest of the shell goes to every model as a topic; "All settings" opens Settings at its page.
    fn panel_events(&mut self, now: u64, acts: &mut Vec<Act>) {
        for (topic, data) in self.panels.take_events() {
            if topic == "settings.open" {
                let page = data.get("page").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string);
                acts.extend(self.open_settings(page.as_deref(), now));
            }
            self.feed(&Input::Topic { topic, data }, now, acts);
        }
        // what was typed goes back, once, to the screen that asked (Settings is the only one that does); it is no topic, so no other model
        // hears it
        for typed in self.panels.take_typed() {
            if self.kinds.settings {
                match typed {
                    Typed::Entered(text) => {
                        for e in self.settings.entered(&text, now) {
                            self.carry(e, now, acts);
                        }
                    }
                    Typed::Cancelled => self.settings.entry_cancelled(),
                }
                self.redraw = true;
            }
        }
    }

    /// Opens Settings, at `page` when one is named: a tile of the quick panel, a card of Home, the compositor's `show`.
    pub fn open_settings(&mut self, page: Option<&str>, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        if self.kinds.settings {
            for e in self.settings.open(page, now) {
                self.carry(e, now, &mut acts);
            }
            self.redraw = true;
            self.after(now, &mut acts);
        }
        acts
    }

    /// Opens the overlay layout editor on `item` (a registry id: `volume`, `mute`, `popup`, `cards`, `toast`; the first when it is none) over
    /// whatever is up, Settings included: the editor is its own screen above it (Settings' row, the compositor's `show`).
    pub fn open_layout(&mut self, item: Option<&str>, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        self.start_layout(item);
        self.after(now, &mut acts);
        acts
    }

    /// What the surfaces the editor stands in for measure at the person's text size.
    fn layout_sizes(&mut self) -> lumatv_placement::Sizes {
        let mut sizes = lumatv_placement::Sizes::new();
        self.sys.sample_sizes(&mut self.kit, &mut sizes);
        self.volume_surface.sample_sizes(&mut self.kit, &mut sizes);
        sizes.set("toast", lumatv_placement::Size::new(560, self.toast_surface.sample_height(&self.kit).ceil() as i32));
        sizes.set("mute", lumatv_placement::Size::new(64, 64));
        sizes
    }

    /// The editor is started with the sizes of the stand-ins it draws, measured as the real surfaces are at the person's text size.
    fn start_layout(&mut self, item: Option<&str>) {
        if !self.kinds.layout {
            return;
        }
        let sizes = self.layout_sizes();
        self.sys.start_layout(item, sizes);
        self.redraw = true;
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
                    // a `show` kept in the state is a press that was answered long ago
                    if topic == "show" {
                        continue;
                    }
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
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("settings") && self.kinds.settings => {
                let page = data.get("page").and_then(Value::as_str);
                for e in self.settings.open(page, now) {
                    self.carry(e, now, &mut acts);
                }
                self.redraw = true;
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("layout") && self.kinds.layout => {
                // `ctl shell show layout ITEM` names the item as the page
                self.start_layout(data.get("item").or_else(|| data.get("page")).and_then(Value::as_str));
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("banner") && self.kinds.banner => {
                // `ctl shell show banner pill|bar|nosignal`: what the QML's development door was, for a TV check
                for e in self.sys.show_banner(data.get("page").and_then(Value::as_str).unwrap_or("pill"), now) {
                    self.carry(e, now, &mut acts);
                }
                self.redraw = true;
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
        // the voice key is held while the person speaks: its press starts the microphone, its release ends it, and nothing else hears it
        if let KeyEvent::Key { name, down, repeat, .. } = event
            && name == "voice"
        {
            if self.kinds.home && !*repeat && (!self.sys.questions_up(&self.kinds) || !*down) {
                for e in self.sys.voice_key(*down, now) {
                    self.carry(e, now, &mut acts);
                }
            }
            self.after(now, &mut acts);
            return acts;
        }
        match event {
            KeyEvent::Key { name, down, repeat, .. } => {
                if *down {
                    self.sys.on_key(&self.kit, &mut self.engine);
                    if self.kinds.cameras && self.cameras.wants() {
                        self.cameras.surface.on_key(&mut self.kit, &mut self.engine);
                    }
                    self.settings.key_taken(&mut self.engine);
                    self.panels.on_key(&self.kit, &mut self.engine);
                }
                // a card that has been up long enough takes OK and Back, and the press' release and repeats: nothing under it sees them
                let key_input = Input::Key { name: name.clone(), down: *down, repeat: *repeat };
                self.cards_took = self.sys.takes_key(&self.kinds, &key_input, now);
                // a question of the system layer (an alert, a PIN prompt) has the keys before Settings does
                let sys_asks = self.settings.is_open() && self.sys.wants(&self.kinds);
                // the entry sheet is over Settings and takes every key while it is up, and the release of a key it took (Settings acts on a
                // release: the one that follows a press the sheet took must not reach the row behind it)
                let entry_up = self.panels.entry_up(&self.kinds);
                let took = self.entry_keys.iter().position(|k| k == name);
                let for_entry = entry_up || took.is_some();
                if *down && entry_up && took.is_none() {
                    self.entry_keys.push(name.clone());
                }
                if !*down && let Some(i) = took {
                    self.entry_keys.remove(i);
                }
                if sys_asks {
                    for e in self.sys.update(&self.kinds, &Input::Key { name: name.clone(), down: *down, repeat: *repeat }, now) {
                        self.carry(e, now, &mut acts);
                    }
                } else if !for_entry {
                    self.feed(&Input::Key { name: name.clone(), down: *down, repeat: *repeat }, now, &mut acts);
                }
                // a panel has the keys unless a question of the system layer or Settings is over it, or a card took the key (the entry sheet is over Settings)
                if !self.sys.wants(&self.kinds) && (for_entry || !self.settings.is_open()) && !self.cards_took {
                    for e in self.panels.key(&self.kinds, name, *down, *repeat, now) {
                        self.carry(e, now, &mut acts);
                    }
                    self.panel_events(now, &mut acts);
                }
            }
            KeyEvent::Pointer { x, y, buttons, visible } => {
                self.cards_took = false;
                let now_ns = self.kit.ui.scene.now_ns();
                let sample = Sample { x: *x, y: *y, buttons: *buttons, visible: *visible };
                // the layout editor and the AI prompt are the whole screen's: nothing under them hears the pointer
                let editor = self.sys.layout_open() || self.sys.ai_up();
                if let Some(target) = self.sys.pointer(&self.kit, &mut self.engine, now_ns, now, sample) {
                    for e in self.sys.press(target) {
                        self.carry(e, now, &mut acts);
                    }
                } else if self.kinds.settings && !self.sys.wants(&self.kinds) && !self.panels.entry_up(&self.kinds) {
                    for e in self.settings.pointer(&mut self.engine, now_ns, sample, now) {
                        self.carry(e, now, &mut acts);
                    }
                }
                if self.kinds.cameras && !(self.kinds.settings && self.settings.is_open()) && !editor {
                    for e in self.cameras.pointer(&mut self.kit, &mut self.engine, now_ns, now, sample) {
                        self.carry(e, now, &mut acts);
                    }
                    self.cameras_asks(now, &mut acts);
                }
                self.feed(&Input::Pointer { x: *x, y: *y, buttons: *buttons, visible: *visible }, now, &mut acts);
                let sample = Sample { x: *x, y: *y, buttons: *buttons, visible: *visible };
                if !editor {
                    for e in self.panels.pointer(&self.kinds, &self.kit, &mut self.engine, now_ns, sample, now) {
                        self.carry(e, now, &mut acts);
                    }
                }
                self.panel_events(now, &mut acts);
            }
            KeyEvent::Closed => {}
        }
        self.cards_took = false;
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
        for e in self.panels.tick(&self.kinds, &self.kit, now) {
            self.carry(e, now, &mut acts);
        }
        self.panel_events(now, &mut acts);
        if self.kinds.home && self.home_surface.tick(now) {
            self.redraw = true;
        }
        if self.kinds.cameras && self.cameras.surface.tick(now) {
            self.redraw = true;
        }
        if self.kinds.settings && self.settings.tick(&mut self.engine, now) {
            self.redraw = true;
        }
        self.after(now, &mut acts);
        acts
    }

    /// Draws what changed and reports when the report changed.
    fn after(&mut self, now: u64, acts: &mut Vec<Act>) {
        // the material in use and the saver: what the moving wallpaper's clip needs to be Home's besides the theme
        if self.kinds.home {
            let (light, saver) = (self.kit.mode == ui::lucent::palette::Mode::Light, self.sys.saver_up(&self.kinds));
            for e in self.home.set_context(light, saver, now) {
                self.carry(e, now, acts);
            }
        }
        // what the panels, Settings and the Cameras viewer have up is what the cards and the pop-up wait for
        let host_busy = self.panels.wants(&self.kinds) || (self.kinds.settings && self.settings.is_open());
        let viewer = self.kinds.cameras && self.cameras.wants();
        for e in self.sys.set_host(&self.kinds, host_busy, viewer, now) {
            self.carry(e, now, acts);
        }
        self.layer_intents(now, acts);
        for e in self.sys.take_effects() {
            self.carry(e, now, acts);
        }
        if self.sys.take_redraw() {
            self.redraw = true;
        }
        if self.panels.take_redraw() {
            self.redraw = true;
        }
        // what the compositor made of the live picture's claim on a video window: the full view follows it
        if self.kinds.cameras && self.cameras.app.set_window(crate::cameras::app::Window::of(self.vwin.grant(crate::cameras::app::CLAIM_ID))) {
            self.redraw = true;
        }
        // the bell's badge counts what the drawer's log holds unread: a toast arrives, or the drawer opens and marks it seen
        if self.kinds.home && self.kinds.drawer && self.home.set_unread(self.panels.unread()) {
            self.redraw = true;
        }
        if self.kinds.settings {
            self.settings_requests(now, acts);
            if self.settings.take_redraw() {
                self.redraw = true;
            }
        }
        if self.redraw {
            self.redraw = false;
            let mut live = false;
            if self.kinds.osd {
                // a mute mark the owner placed drifts about its place, clear of the cards that are up
                let avoid = self.sys.avoid(&self.kinds);
                self.volume.set_placed(self.volume_surface.mark_places(&avoid));
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
            let now_ns = self.kit.ui.scene.now_ns();
            match self.panels.apply(&self.kinds, &mut self.kit, &mut self.engine, now, now_ns) {
                Ok(on) => live |= on,
                Err(e) => acts.push(Act::Log(e)),
            }
            // the places of the switcher's cards, for the compositor to move its pictures to
            for e in self.panels.take_drawn() {
                self.carry(e, now, acts);
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
            if self.kinds.settings {
                let now_ns = self.kit.ui.scene.now_ns();
                let over_video = self.sys.app_front();
                match self.settings.apply(&mut self.kit, &mut self.engine, over_video, now, now_ns) {
                    Ok(on) => live |= on,
                    Err(e) => acts.push(Act::Log(format!("settings: {e}"))),
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

    /// What the cards and the pop-up ask that is not theirs to do: OK on the camera card opens the Cameras app's full view, OK on a card with no
    /// action of its own opens the notification drawer.
    fn layer_intents(&mut self, now: u64, acts: &mut Vec<Act>) {
        for i in self.sys.take_intents() {
            match i {
                crate::system::layer::Intent::OpenCamera(camera) if self.kinds.cameras => {
                    for e in self.cameras.open_full(&camera, now) {
                        self.carry(e, now, acts);
                    }
                    self.redraw = true;
                }
                crate::system::layer::Intent::OpenCamera(camera) => acts.push(Act::Log(format!("camera card: the full view of {camera} is not this process's"))),
                crate::system::layer::Intent::OpenDrawer => {
                    let mut out = Vec::new();
                    self.panels.show(Which::Drawer, false, &self.kinds, now, &mut out);
                    for e in out {
                        self.carry(e, now, acts);
                    }
                }
                // a banner's Inputs and Picture buttons: this process's input picker and quick panel when they are kinds of it, else what the
                // compositor opens for the gear and Input when LumaTV's panels are not up (`system_ui.rs`: the Home Hub, LG's Settings)
                crate::system::layer::Intent::OpenPicker => self.open_panel_or_app(Which::Picker, self.kinds.picker, "com.webos.app.homeconnect", now, acts),
                crate::system::layer::Intent::OpenQuick => self.open_panel_or_app(Which::Quick, self.kinds.quick, "com.palm.app.settings", now, acts),
            }
        }
    }

    /// Opens `panel` when it is a kind of this process, else LG's `app` (what the compositor's own routes launch for the same key).
    fn open_panel_or_app(&mut self, panel: Which, is_kind: bool, app: &str, now: u64, acts: &mut Vec<Act>) {
        if is_kind {
            let mut out = Vec::new();
            self.panels.show(panel, false, &self.kinds, now, &mut out);
            for e in out {
                self.carry(e, now, acts);
            }
        } else {
            self.carry(Effect::Luna { tag: crate::system::layer::LAUNCH_TAG, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": app}), subscribe: false }, now, acts);
        }
    }

    /// The report the compositor reads: what this process can show (the layout editor, when it is a kind), whether its layer wants
    /// the keys (an alert, the input popup or a PIN prompt is up), the keys its cards take while an app has the focus, whether it has
    /// something on screen, and its state for the gate.
    pub fn report(&self) -> Value {
        let mut can: Vec<&str> = Vec::new();
        if self.kinds.banner {
            can.push("banner");
        }
        if self.kinds.layout {
            can.push("layout");
        }
        if self.kinds.cameras {
            can.push("cameras");
        }
        if self.kinds.settings {
            can.push("settings");
        }
        can.extend(self.panels.can(&self.kinds));
        // Home in front wants the keys too: the compositor sends every key of the remote to the layer that wants them
        let wants = self.sys.wants(&self.kinds) || self.panels.wants(&self.kinds) || (self.kinds.home && self.home.active) || (self.kinds.cameras && self.cameras.wants()) || (self.kinds.settings && self.settings.wants());
        let mut takes = self.sys.takes(&self.kinds);
        if self.kinds.home {
            // the keys the compositor has no name for until it is told they are the shell's: the voice key while the helper can listen, the
            // on-screen remote's and Search's while Home is in front
            if self.sys.takes_voice() {
                takes.push("voice");
            }
            if self.home.active {
                takes.extend(["screenremote", "search"]);
            }
        }
        json!({"can": can, "wants": wants, "takes": takes, "live": self.live, "state": self.state()})
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
        s["panels"] = self.panels.state(&self.kinds);
        if self.kinds.home {
            s["home"] = self.home.state();
        }
        if self.kinds.cameras {
            s["cameras"] = self.cameras.state();
        }
        if let Some(v) = self.vwin.state() {
            s["vwin"] = v;
        }
        if self.kinds.settings {
            for (k, val) in self.settings.state().as_object().into_iter().flatten() {
                s[k] = val.clone();
            }
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
        let settings = if self.kinds.settings { self.settings.due_at() } else { None };
        [due, self.sys.due_at(&self.kinds), self.panels.due_at(&self.kinds), cameras, settings, self.vwin.due_at()].into_iter().flatten().min()
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
        for (k, v) in self.sys.geometry(&self.kinds, &self.kit, &self.engine).as_object().into_iter().flatten() {
            g[k] = v.clone();
        }
        for (k, v) in self.panels.geometry(&self.kinds, &self.engine).as_object().into_iter().flatten() {
            g[k] = v.clone();
        }
        if self.kinds.home {
            g["home"] = self.home_surface.geometry(&self.home);
        }
        if self.kinds.cameras {
            g["cameras"] = self.cameras.geometry();
        }
        if self.kinds.settings {
            for (k, v) in self.settings.geometry().as_object().into_iter().flatten() {
                g[k] = v.clone();
            }
        }
        g
    }

    /// The layout and contrast lint of what the layer has on the screen (`--eval lint`): an object per finding.
    pub fn lint(&mut self) -> Value {
        let mut lint = crate::kit::lint::Lint::new();
        // the entry sheet's dim hides the screen it is over: only the sheet is read
        if self.panels.entry_up(&self.kinds) {
            self.panels.feed_lint(&self.kinds, &mut self.kit, &mut lint);
            return json!(lint.findings().iter().map(|f| json!({"kind": f.kind.name(), "subject": f.subject, "detail": f.detail})).collect::<Vec<_>>());
        }
        self.sys.feed_lint(&self.kinds, &mut self.kit, &mut lint);
        // what Home draws around itself stands over Home like the entry sheet does over Settings: while one is up only it is read
        if self.kinds.home && !self.sys.extras_up() {
            lint = self.home_surface.lint(&mut self.kit, &self.home);
        }
        if self.kinds.cameras {
            self.cameras.surface.feed_lint(&mut self.kit, &mut lint);
        }
        self.panels.feed_lint(&self.kinds, &mut self.kit, &mut lint);
        if self.kinds.settings {
            self.settings.feed_lint(&mut self.kit, &mut lint);
        }
        json!(lint.findings().iter().map(|f| json!({"kind": f.kind.name(), "subject": f.subject, "detail": f.detail})).collect::<Vec<_>>())
    }

    /// The volume model's own account (`--eval volume`).
    pub fn debug(&self) -> Value {
        self.volume.debug()
    }

    /// The theme or the text size changed: the surfaces are made again at the next draw, and the layout editor, when it is open, measures the
    /// surfaces it stands in for again (its stand-ins follow the person's text size, not the size they had when it was opened).
    pub fn restyle(&mut self) {
        self.volume_surface.restyle();
        self.toast_surface.restyle();
        self.sys.restyle(&mut self.engine);
        self.panels.restyle(&mut self.engine);
        self.home_surface.restyle();
        self.cameras.restyle(&mut self.engine, self.kit.text_scale);
        self.settings.restyle();
        if self.kinds.layout && self.sys.layout_open() {
            let sizes = self.layout_sizes();
            self.sys.set_layout_sizes(sizes);
        }
        self.redraw = true;
    }

    /// The Home model, for the harness's `--eval` and the tests.
    pub fn home(&self) -> &Home {
        &self.home
    }

    /// What the Cameras app asked of the shell: its grid's `Settings` button (the app closes first) opens Settings at Cameras (a page of Home & look).
    fn cameras_asks(&mut self, now: u64, acts: &mut Vec<Act>) {
        for intent in self.cameras.take_intents() {
            match intent {
                crate::cameras::app::Intent::OpenSettings if self.kinds.settings => {
                    for e in self.settings.open(Some(CAMERAS_SETTINGS_PAGE), now) {
                        self.carry(e, now, acts);
                    }
                    self.redraw = true;
                }
                crate::cameras::app::Intent::OpenSettings => acts.push(Act::Log("cameras ask for settings".into())),
            }
        }
    }

    /// The Cameras app, for the screens that open it (the pop-up's OK, the quick panel's tile).
    pub fn cameras(&mut self) -> &mut Cameras {
        &mut self.cameras
    }

    /// What the screens want on the display's video windows now (the claims the compositor's `vwin` is asked for): the Cameras full view's live picture.
    /// A screen with nothing live wants nothing, and the claim it had is released. A screen that wants a picture-in-picture adds its `Pip::request()` here.
    fn vwin_wants(&self) -> Vec<Value> {
        let live = if self.kinds.cameras { self.cameras.app.live_claim().map(|c| c.request()) } else { None };
        live.into_iter().collect()
    }

    /// What Settings asked of the shell: to close, to launch LG's Settings app, to open the layout editor; the theme it wrote becomes the
    /// look in force.
    fn settings_requests(&mut self, now: u64, acts: &mut Vec<Act>) {
        for r in self.settings.take_requests() {
            match r {
                ShellRequest::Close => {
                    for e in self.settings.close(now) {
                        self.carry(e, now, acts);
                    }
                }
                ShellRequest::Launch { app, params } => acts.push(Act::Channel(Outbound {
                    method: "POST",
                    path: "/v1/luna".into(),
                    body: Some(json!({"tag": crate::settings::src::TAG_END, "uri": "luna://com.webos.applicationManager/launch", "payload": {"id": app, "params": params}, "subscribe": false})),
                    tag: None,
                })),
                ShellRequest::Layout { item } => {
                    if self.kinds.layout {
                        self.start_layout(item.as_deref());
                    } else {
                        acts.push(Act::Log(format!("settings: the layout editor was asked for ({item:?}) and is not this process's")));
                    }
                }
                ShellRequest::HomeLayout(layout) => {
                    let mut out = Vec::new();
                    self.home.set_layout_from_settings(&layout, &mut out);
                    for e in out {
                        self.carry(e, now, acts);
                    }
                }
                ShellRequest::EditHome => {
                    for e in self.settings.close(now) {
                        self.carry(e, now, acts);
                    }
                    let mut out = Vec::new();
                    self.home.edit_home("", &mut out);
                    for e in out {
                        self.carry(e, now, acts);
                    }
                }
                ShellRequest::Text { prompt, text, secret } => {
                    let mut out = Vec::new();
                    if self.panels.ask_text(&prompt, &text, secret, &self.kinds, now, &mut out) {
                        for e in out {
                            self.carry(e, now, acts);
                        }
                    } else {
                        // nothing here can type: the question is let go, so Settings does not wait for a keyboard that is not there
                        self.settings.entry_cancelled();
                        acts.push(Act::Log("settings: text was asked for and the entry kind is not on".into()));
                    }
                }
            }
        }
        if self.settings_layout != self.home.layout_text {
            self.settings_layout = self.home.layout_text.clone();
            self.settings.set_layout(self.home.layout.clone());
        }
        if let Some(file) = self.settings.take_theme_file() {
            let theme = self.reload_theme(&file);
            // the saver's own numbers (clockLevel, dim, style, own) and the placements are in it
            for e in self.sys.set_theme(&theme, now) {
                self.carry(e, now, acts);
            }
        }
    }

    /// The owner placed the pop-ups somewhere else (`layout.overlays`): every surface that has a place reads it from now on.
    fn set_places(&mut self, theme: &Value) {
        let user = lumatv_placement::Registry::shipped().user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        self.volume_surface.set_user(user.clone());
        self.toast_surface.set_user(user.clone());
        self.sys.set_user(user);
    }

    /// The owner's theme file changed (Settings wrote it): the look in force is the shipped theme under it. Returns the merged theme.
    pub fn reload_theme(&mut self, file: &Value) -> Value {
        let tokens = lumatv_tokens::Tokens::from_json(file);
        let mode = match tokens.material {
            lumatv_tokens::Material::Dark => ui::lucent::palette::Mode::Dark,
            lumatv_tokens::Material::Light => ui::lucent::palette::Mode::Light,
            lumatv_tokens::Material::Auto { .. } => self.kit.mode,
        };
        let scale = file.pointer("/type/scale").and_then(Value::as_f64).map_or(self.kit.text_scale, |s| (s as f32).clamp(1.0, 2.0));
        let theme = tokens.json().clone();
        self.kit.retheme(tokens, mode, scale);
        self.set_places(&theme);
        self.settings.set_theme(theme.clone());
        self.restyle();
        theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    fn shell() -> (ui::testing::Harness, Shell) {
        let (h, kit) = testkit::kit(1.0);
        let shell = Shell::new(kit, Kinds::parse("cameras"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        (h, shell)
    }

    #[test]
    fn the_ai_prompt_is_over_everything_has_the_keys_and_the_pointer_and_no_key_or_click_makes_the_shell_say_anything_of_an_answer() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("ai,home,settings,quick"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        shell.start();
        assert_eq!(shell.report()["wants"], false);
        let doc = json!({"id": "a1", "kind": "tv_standby", "assistant": {"id": "c", "name": "Cody"}, "title": "Turn the TV off", "detail": "The TV goes to standby.",
                         "reason": "Nitin said good night", "shownAt": 1_000, "expiresAt": 31_000, "allow": "OK", "deny": "Back"});
        let acts = shell.helper(&Event::Topic { topic: "ai.prompt".into(), data: doc }, 1_100);
        assert_eq!(shell.report()["wants"], true, "the compositor sends every key to this process while the prompt is up");
        assert_eq!(shell.state()["ai"]["up"], true);
        assert!(shell.is_live(), "and it is drawn");
        assert!(acts.iter().any(|a| matches!(a, Act::Channel(o) if o.path == "/v1/ui")), "the compositor is told");
        // no key and no click says anything to the helper, the luna bridge or the compositor but the report
        let silent = |acts: &[Act]| acts.iter().all(|a| matches!(a, Act::Log(_)) || matches!(a, Act::Channel(o) if o.path == "/v1/ui"));
        for name in ["ok", "back", "up", "down", "left", "right", "1"] {
            for down in [true, false] {
                let acts = shell.key(&KeyEvent::Key { name: name.into(), down, repeat: false, seat: 0 }, 1_200);
                assert!(silent(&acts), "{name} down={down}: {acts:?}");
            }
        }
        for buttons in [0, 1, 0] {
            let acts = shell.key(&KeyEvent::Pointer { x: 960.0, y: 540.0, buttons, visible: true }, 1_300);
            assert!(silent(&acts), "a click answers nothing: {acts:?}");
        }
        assert_eq!(shell.state()["ai"]["up"], true, "still up: only the helper takes it away");
        shell.helper(&Event::Topic { topic: "ai.prompt".into(), data: Value::Null }, 2_000);
        assert_eq!(shell.report()["wants"], false);
        assert_eq!(shell.state()["ai"], json!({"up": false}));
    }

    #[test]
    fn the_layout_editor_measures_its_stand_ins_again_when_the_text_size_changes_while_it_is_open() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("layout"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        shell.open_layout(None, 1_000);
        let height_of = |s: &Shell, id: &str| -> f64 {
            s.geometry()["layout"]["items"].as_array().and_then(|items| items.iter().find(|i| i["id"] == id)).and_then(|i| i["h"].as_f64()).unwrap_or(0.0)
        };
        let (cards, popup) = (height_of(&shell, "cards"), height_of(&shell, "popup"));
        assert!(cards > 0.0 && popup > 0.0, "the editor is open and drew its stand-ins: {}", shell.geometry()["layout"]);
        shell.reload_theme(&json!({"type": {"scale": 2.0}}));
        shell.tick(1_100);
        assert!(height_of(&shell, "cards") > cards, "the cards' stand-in is taller at 200 % text, with the editor still open");
        assert!(height_of(&shell, "popup") > popup, "so is the camera window's");
        assert_eq!(shell.state()["layout"], json!({"open": true}), "the editor stayed open through it");
    }

    /// A Home shell on `kinds` (always with `home`) with the launch points of the dock and Home in front.
    fn home_shell(kinds: &str) -> (ui::testing::Harness, Shell) {
        home_shell_with(kinds, &json!({}))
    }

    /// The same with the owner's theme `patch` over the shipped one, and launch points two of which are the library's only.
    fn home_shell_with(kinds: &str, patch: &Value) -> (ui::testing::Harness, Shell) {
        let (h, kit) = testkit::kit(1.0);
        let mut theme = lumatv_tokens::Tokens::shipped().json().clone();
        lumatv_tokens::merge(&mut theme, patch);
        let mut shell = Shell::new(kit, Kinds::parse(kinds), 2000, &testkit::module_dir().join("icons/audio"), &theme, &json!({})).expect("a shell");
        shell.start();
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1_000);
        let mut lps = vec![json!({"id": "com.webos.app.discovery", "title": "Apps", "unmovable": true, "icon": "/a.png"})];
        for (id, t) in [("netflix", "Netflix"), ("youtube", "YouTube"), ("plex", "Plex"), ("extra.a", "Extra A"), ("extra.b", "Extra B")] {
            lps.push(json!({"id": id, "title": t, "icon": format!("/{id}.png"), "iconColor": "#101010"}));
        }
        shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": crate::home::model::LP_TAG, "payload": {"launchPoints": lps}}) }, 1_010);
        (h, shell)
    }

    fn key(shell: &mut Shell, name: &str, at: u64) -> Vec<Act> {
        let mut acts = shell.key(&KeyEvent::Key { name: name.into(), down: true, repeat: false, seat: 0 }, at);
        acts.extend(shell.key(&KeyEvent::Key { name: name.into(), down: false, repeat: false, seat: 0 }, at + 30));
        acts
    }

    fn luna_calls(acts: &[Act], method: &str) -> Vec<Value> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Channel(o) if o.path == "/v1/luna" => o.body.clone(),
                _ => None,
            })
            .filter(|b| b["uri"].as_str().is_some_and(|u| u.ends_with(method)))
            .collect()
    }

    #[test]
    fn an_app_that_opens_from_home_shows_the_launch_screen_after_the_compositors_wait_and_tells_it_where_the_icon_is_and_it_goes_when_the_app_has_the_front() {
        let (_h, mut shell) = home_shell("home");
        shell.channel(&Event::Topic { topic: "transitions".into(), data: json!({"on": true, "reduced": false}) }, 1_020);
        for (i, k) in ["down", "down", "down"].into_iter().enumerate() {
            key(&mut shell, k, 2_000 + i as u64 * 100);
        }
        let acts = key(&mut shell, "ok", 2_400);
        assert_eq!(luna_calls(&acts, "/launch").len(), 1, "the app is asked for");
        let origin = luna_calls(&acts, "/setLaunchOrigin");
        assert_eq!(origin.len(), 1, "the compositor is told where the tile is at once: {acts:?}");
        assert_eq!(origin[0]["payload"]["appId"], "netflix");
        assert!(origin[0]["payload"]["tile"]["width"].as_f64().unwrap() > 100.0 && origin[0]["payload"]["tile"]["radius"].as_f64().unwrap() > 0.0);
        assert_eq!(shell.state()["extras"]["splashWaiting"], true, "the screen waits for the card to be what is seen");
        assert!(shell.state()["extras"]["splash"].is_null() && shell.geometry()["extras"]["splash"].is_null(), "nothing drawn yet");
        // the wait is over and the app has no window: the launch screen shows, with the icon where the card is to grow from next
        let acts = shell.tick(2_850);
        assert_eq!(shell.state()["extras"]["splash"]["name"], "Netflix");
        assert_eq!(shell.geometry()["extras"]["splash"]["icon"]["w"], 180.0, "and it is drawn");
        let acts: Vec<Act> = acts.into_iter().chain(shell.tick(2_950)).collect();
        let from = luna_calls(&acts, "/setLaunchOrigin");
        assert_eq!(from.len(), 1, "one more note: where the icon is: {acts:?}");
        assert_eq!(from[0]["payload"]["from"]["width"], 180.0);
        assert_eq!(from[0]["payload"]["from"]["radius"], 38.0);
        // the app has the front: Home is behind it and the launch screen is gone
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false, "covered": true}) }, 3_100);
        assert!(shell.state()["extras"]["splash"].is_null() && shell.geometry()["extras"]["splash"].is_null());
    }

    #[test]
    fn without_the_compositors_transitions_the_launch_screen_is_there_at_once_as_it_always_was() {
        let (_h, mut shell) = home_shell("home");
        for (i, k) in ["down", "down", "down"].into_iter().enumerate() {
            key(&mut shell, k, 2_000 + i as u64 * 100);
        }
        let acts = key(&mut shell, "ok", 2_400);
        assert!(luna_calls(&acts, "/setLaunchOrigin").is_empty(), "nothing grows out of a tile");
        assert_eq!(shell.state()["extras"]["splash"]["name"], "Netflix");
    }

    fn quick_access() -> Value {
        json!({"remote": {"quickAccess": {"on": true, "slots": {}}}, "apps": {"dockHide": ["extra.a", "extra.b"]}})
    }

    #[test]
    fn a_digit_with_nothing_behind_it_opens_the_question_which_has_every_key_and_the_pointer_and_its_answer_is_kept_and_opens_the_app() {
        let (_h, mut shell) = home_shell_with("home,settings", &quick_access());
        key(&mut shell, "9", 2_000);
        assert_eq!(shell.state()["extras"]["ask"]["digit"], "9");
        assert_eq!(shell.state()["extras"]["ask"]["picks"], json!(["extra.a", "extra.b"]));
        assert_eq!(shell.geometry()["extras"]["ask"]["picks"].as_array().map(Vec::len), Some(3), "two apps and More");
        assert_eq!(shell.report()["wants"], true);
        // a digit is not an answer and is not Home's while the question is up; the arrows move the choice
        let acts = key(&mut shell, "2", 2_100);
        assert!(luna_calls(&acts, "/launch").is_empty(), "{acts:?}");
        key(&mut shell, "right", 2_200);
        assert_eq!(shell.state()["extras"]["ask"]["index"], 1);
        key(&mut shell, "left", 2_300);
        let acts = key(&mut shell, "ok", 2_400);
        let launched = luna_calls(&acts, "/launch");
        assert_eq!(launched.len(), 1, "the app opens: {acts:?}");
        assert_eq!(launched[0]["payload"]["id"], "extra.a");
        assert!(shell.state()["extras"]["ask"].is_null(), "answered");
        assert_eq!(shell.state()["extras"]["toast"]["title"], "Extra A");
        assert_eq!(shell.state()["home"]["launching"], "Extra A");
    }

    #[test]
    fn back_leaves_the_question_with_nothing_opened_and_more_is_settings_at_its_remote_page() {
        let (_h, mut shell) = home_shell_with("home,settings", &quick_access());
        key(&mut shell, "9", 2_000);
        let acts = key(&mut shell, "back", 2_100);
        assert!(shell.state()["extras"]["ask"].is_null() && luna_calls(&acts, "/launch").is_empty());
        key(&mut shell, "9", 2_200);
        key(&mut shell, "right", 2_300);
        key(&mut shell, "right", 2_400);
        key(&mut shell, "ok", 2_500);
        assert_eq!(shell.state()["settings"]["open"], true);
        assert_eq!(shell.state()["settings"]["page"], "remote");
    }

    #[test]
    fn a_click_on_a_card_of_the_question_answers_it_and_nothing_under_the_question_hears_the_pointer() {
        let (_h, mut shell) = home_shell_with("home", &quick_access());
        key(&mut shell, "9", 2_000);
        let card = shell.geometry()["extras"]["ask"]["picks"][1].clone();
        let (x, y) = ((card["x"].as_f64().unwrap() + card["w"].as_f64().unwrap() / 2.0) as f32, (card["y"].as_f64().unwrap() + 20.0) as f32);
        let mut acts = Vec::new();
        for (i, buttons) in [0, 0, 1, 0].into_iter().enumerate() {
            acts.extend(shell.key(&KeyEvent::Pointer { x, y, buttons, visible: true }, 2_100 + i as u64 * 50));
            shell.tick(2_100 + i as u64 * 50);
        }
        let launched = luna_calls(&acts, "/launch");
        assert_eq!(launched.len(), 1, "{acts:?}");
        assert_eq!(launched[0]["payload"]["id"], "extra.b", "the second card");
    }

    #[test]
    fn the_remote_key_opens_the_on_screen_remote_which_presses_the_micom_keycodes_and_has_no_key_of_the_prompts_and_back_closes_it() {
        let (_h, mut shell) = home_shell("home");
        key(&mut shell, "screenremote", 2_000);
        assert_eq!(shell.state()["extras"]["remote"], "rewind");
        assert_eq!(shell.geometry()["extras"]["remote"]["keys"].as_array().map(Vec::len), Some(21));
        // the D-pad walks it: Right to Play, OK presses it
        key(&mut shell, "right", 2_100);
        let acts = key(&mut shell, "ok", 2_200);
        let sent = luna_calls(&acts, "/sendKeycode");
        assert_eq!(sent.len(), 1, "{acts:?}");
        assert_eq!((sent[0]["uri"].as_str(), sent[0]["payload"]["keycode"].as_u64()), (Some("luna://com.webos.service.micomservice/sendKeycode"), Some(176)));
        assert_eq!(shell.state()["extras"]["remote"], "play");
        // the keys of Home are not Home's while it is up
        let acts = key(&mut shell, "3", 2_300);
        assert!(luna_calls(&acts, "/launch").is_empty());
        key(&mut shell, "back", 2_400);
        assert!(shell.state()["extras"]["remote"].is_null() && shell.geometry()["extras"]["remote"].is_null());
        // and the key again toggles it
        key(&mut shell, "screenremote", 2_500);
        key(&mut shell, "screenremote", 2_600);
        assert!(shell.state()["extras"]["remote"].is_null());
    }

    #[test]
    fn a_click_on_a_key_of_the_on_screen_remote_presses_it_and_never_reaches_the_home_under_it() {
        let (_h, mut shell) = home_shell("home");
        key(&mut shell, "screenremote", 2_000);
        let keys = shell.geometry()["extras"]["remote"]["keys"].clone();
        let f = keys.as_array().unwrap().iter().find(|k| k["name"] == "blue").unwrap()["frame"].clone();
        let (x, y) = ((f["x"].as_f64().unwrap() + 30.0) as f32, (f["y"].as_f64().unwrap() + 30.0) as f32);
        let mut acts = Vec::new();
        for (i, buttons) in [0, 0, 1, 0].into_iter().enumerate() {
            acts.extend(shell.key(&KeyEvent::Pointer { x, y, buttons, visible: true }, 2_100 + i as u64 * 50));
            shell.tick(2_100 + i as u64 * 50);
        }
        let sent = luna_calls(&acts, "/sendKeycode");
        assert_eq!(sent.len(), 1, "{acts:?}");
        assert_eq!(sent[0]["payload"]["keycode"], 97, "blue");
        assert!(luna_calls(&acts, "/launch").is_empty(), "the dock under the sheet heard nothing");
    }

    fn helper_answer(shell: &mut Shell, tag: u64, body: Value, at: u64) -> Vec<Act> {
        shell.helper(&Event::Answer { tag, status: 200, body }, at)
    }

    fn helper_calls(acts: &[Act], path: &str) -> usize {
        acts.iter().filter(|a| matches!(a, Act::Helper(o) if o.path == path)).count()
    }

    #[test]
    fn holding_the_voice_key_listens_the_bar_and_the_mark_show_letting_go_stops_and_the_words_open_the_search() {
        let (_h, mut shell) = home_shell("home");
        assert!(!shell.report()["takes"].as_array().unwrap().iter().any(|t| t == "voice"), "the helper cannot listen yet: the key stays LG's");
        shell.helper(&Event::Topic { topic: "voice".into(), data: json!({"ready": true, "state": "idle"}) }, 1_500);
        let takes: Vec<String> = shell.report()["takes"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
        assert!(takes.contains(&"voice".to_string()) && takes.contains(&"screenremote".to_string()) && takes.contains(&"search".to_string()), "{takes:?}");
        let acts = shell.key(&KeyEvent::Key { name: "voice".into(), down: true, repeat: false, seat: 0 }, 2_000);
        assert_eq!(helper_calls(&acts, "/v1/voice/listen"), 1, "{acts:?}");
        assert_eq!((shell.state()["extras"]["voice"].as_str(), shell.state()["extras"]["mic"].as_bool()), (Some("listening"), Some(true)));
        assert_eq!(shell.geometry()["extras"]["voice"]["bar"]["caption"], "Listening");
        assert!(shell.geometry()["extras"]["voice"]["mark"].is_object(), "the microphone is open");
        let acts = helper_answer(&mut shell, crate::system::extras::VOICE_TAG + 1, json!({"ok": true, "session": 7}), 2_050);
        assert_eq!(helper_calls(&acts, "/v1/voice/stop"), 0, "the key is still held");
        let acts = shell.key(&KeyEvent::Key { name: "voice".into(), down: false, repeat: false, seat: 0 }, 2_600);
        assert_eq!(helper_calls(&acts, "/v1/voice/stop"), 1, "{acts:?}");
        assert_eq!(shell.state()["extras"]["voice"], "working");
        shell.helper(&Event::Topic { topic: "voice".into(), data: json!({"ready": true, "state": "heard", "session": 7, "text": "the news"}) }, 3_000);
        assert_eq!(shell.state()["extras"]["voice"], "");
        assert_eq!(shell.state()["home"]["layer"], "search", "the words go to Search and nothing acts on them");
        assert_eq!(shell.state()["extras"]["mic"], false);
    }

    #[test]
    fn back_ends_the_microphone_and_a_voice_key_nobody_can_answer_does_nothing() {
        let (_h, mut shell) = home_shell("home");
        // the helper cannot listen: the key's press asks the helper and is told why, the bar says so and goes by itself
        shell.helper(&Event::Up(true), 1_400);
        shell.helper(&Event::Topic { topic: "voice".into(), data: json!({"ready": false, "reason": "voice_off"}) }, 1_500);
        shell.key(&KeyEvent::Key { name: "voice".into(), down: true, repeat: false, seat: 0 }, 2_000);
        helper_answer(&mut shell, crate::system::extras::VOICE_TAG + 1, json!({"ok": false}), 2_050);
        assert_eq!(shell.state()["extras"]["voice"], "failed");
        assert_eq!(shell.state()["extras"]["voiceWords"], "Voice search is switched off");
        shell.tick(2_050 + 2_700);
        assert_eq!(shell.state()["extras"]["voice"], "");
        // listening, then Back
        shell.helper(&Event::Topic { topic: "voice".into(), data: json!({"ready": true}) }, 6_000);
        shell.key(&KeyEvent::Key { name: "voice".into(), down: true, repeat: false, seat: 0 }, 6_100);
        let acts = key(&mut shell, "back", 6_200);
        assert_eq!(helper_calls(&acts, "/v1/voice/cancel"), 1, "{acts:?}");
        assert_eq!((shell.state()["extras"]["voice"].as_str(), shell.state()["extras"]["mic"].as_bool()), (Some(""), Some(false)));
    }

    #[test]
    fn a_story_opens_the_mini_frame_out_of_its_card_the_web_half_is_launched_into_its_interior_and_back_closes_both() {
        let (_h, mut shell) = home_shell("home");
        let news = json!({"doc": {"fetchedAt": 1, "items": [{"tag": "WORLD", "src": "BBC News", "title": "A story", "summary": "", "link": "https://news.example/a", "pubMs": 1}]}, "state": "fresh"});
        shell.helper(&Event::Topic { topic: "news".into(), data: news }, 1_500);
        key(&mut shell, "down", 2_000);
        for i in 0..6 {
            if shell.state()["home"]["widgetFocusId"] == "news" {
                break;
            }
            key(&mut shell, "right", 2_100 + i * 100);
        }
        assert_eq!(shell.state()["home"]["widgetFocusId"], "news", "{}", shell.state()["home"]);
        key(&mut shell, "ok", 3_000);
        let acts = key(&mut shell, "ok", 3_200);
        assert!(luna_calls(&acts, "/launch").is_empty(), "the browser is not opened: the frame is");
        assert_eq!(shell.state()["extras"]["frame"]["title"], "A story");
        assert_eq!(shell.state()["extras"]["frame"]["landed"], false);
        let g = shell.geometry()["extras"]["frame"].clone();
        assert_eq!((g["x"].as_f64(), g["y"].as_f64(), g["w"].as_f64(), g["h"].as_f64()), (Some(200.0), Some(150.0), Some(1520.0), Some(860.0)));
        // it has landed: the web half is launched into the room the frame leaves clear
        let acts = shell.tick(3_200 + 300);
        let launched = luna_calls(&acts, "/launch");
        assert_eq!(launched.len(), 1, "{acts:?}");
        assert_eq!(launched[0]["payload"]["id"], crate::system::extras::FRAME_APP);
        assert_eq!(launched[0]["payload"]["params"]["rect"], json!({"x": 232.0, "y": 214.0, "w": 1456.0, "h": 764.0}));
        assert_eq!(launched[0]["payload"]["params"]["url"], "https://news.example/a");
        assert_eq!(shell.geometry()["extras"]["frame"]["title"], "A story", "and the words are drawn");
        // the web half is up: the loading ring goes after its grace
        shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": crate::system::extras::RUNNING_TAG, "payload": {"running": [{"id": crate::system::extras::FRAME_APP}]}}) }, 3_600);
        shell.tick(4_000);
        assert_eq!(shell.state()["extras"]["frame"]["loading"], false);
        // Back closes the frame and the web half with it
        let acts = key(&mut shell, "back", 4_100);
        assert_eq!(luna_calls(&acts, "/closeByAppId").len(), 1, "{acts:?}");
        assert!(shell.state()["extras"]["frame"].is_null());
        shell.tick(4_100 + 400);
        assert!(shell.geometry()["extras"]["frame"].is_null(), "it shrank back and is gone");
    }

    #[test]
    fn a_model_may_read_and_write_the_data_directory_and_the_players_mailbox_and_nothing_else() {
        let (_h, mut shell) = shell();
        shell.dirs(&testkit::module_dir(), std::path::Path::new("/data/lumatv"));
        assert_eq!(shell.file_of("theme/theme.json").unwrap(), PathBuf::from("/data/lumatv/theme/theme.json"));
        assert_eq!(shell.file_of("/tmp/lumatv-vplane/req.json").unwrap(), PathBuf::from("/tmp/lumatv-vplane/req.json"));
        assert_eq!(shell.file_of("/tmp/lumatv-vplane/status.json").unwrap(), PathBuf::from("/tmp/lumatv-vplane/status.json"));
        for bad in ["/etc/passwd", "/tmp/other/req.json", "/tmp/lumatv-vplane/", "/tmp/lumatv-vplane/../x", "/tmp/lumatv-vplane/a/b", "../x", "theme/../../x", "/tmp/lumatv-vplane-x/req.json"] {
            assert!(shell.file_of(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_mailbox_is_the_one_the_player_and_the_qml_use() {
        let brand: Value = serde_json::from_str(include_str!("../../../brand.json")).expect("brand.json");
        assert_eq!(crate::home::wallvideo::MAILBOX, format!("/tmp/{}-vplane", brand["shortId"].as_str().unwrap()));
    }

    #[test]
    fn a_search_answer_that_is_a_page_of_settings_opens_that_page_in_the_settings_kind() {
        let (_h, mut shell) = home_shell("home,settings");
        key(&mut shell, "search", 2_000);
        assert_eq!(shell.state()["home"]["layer"], "search");
        shell.channel(&Event::Topic { topic: "keyboard.text".into(), data: json!({"text": "netw"}) }, 2_100);
        let answers = shell.state()["home"]["search"]["answers"].clone();
        let page = answers.as_array().unwrap().iter().position(|a| a["kind"] == "page" && a["id"] == "network").expect("Settings' Network page is an answer");
        // the answer is a stop of the arrows: up from the keyboard reaches the answers, down walks them
        for i in 0..6 {
            if shell.state()["home"]["search"]["index"].as_u64().is_some_and(|i| i >= 43) {
                break;
            }
            key(&mut shell, "up", 2_200 + i * 100);
        }
        for i in 0..4 {
            let at = shell.state()["home"]["search"]["index"].as_u64().unwrap();
            if at == 43 + page as u64 {
                break;
            }
            key(&mut shell, if at > 43 + page as u64 { "left" } else { "right" }, 3_000 + i * 100);
        }
        assert_eq!(shell.state()["home"]["search"]["index"], 43 + page as u64, "{}", shell.state()["home"]["search"]);
        key(&mut shell, "ok", 4_000);
        assert_eq!(shell.state()["settings"]["open"], true);
        assert_eq!(shell.state()["settings"]["page"], "network");
        assert_eq!(shell.state()["home"]["layer"], "", "Home's sheet is closed behind it");
    }

    #[test]
    fn while_the_ai_prompt_is_up_no_key_opens_anything_of_homes_extras_and_the_mic_key_is_swallowed() {
        let (_h, kit) = testkit::kit(1.0);
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let mut shell = Shell::new(kit, Kinds::parse("home,ai"), 2000, &testkit::module_dir().join("icons/audio"), &theme, &json!({})).expect("a shell");
        shell.start();
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1_000);
        shell.helper(&Event::Up(true), 1_050);
        shell.helper(&Event::Topic { topic: "voice".into(), data: json!({"ready": true}) }, 1_100);
        let doc = json!({"id": "a1", "kind": "tv_standby", "assistant": {"id": "c", "name": "Cody"}, "title": "Turn the TV off", "detail": "The TV goes to standby.",
                         "reason": "Nitin said good night", "shownAt": 1_000, "expiresAt": 31_000, "allow": "OK", "deny": "Back"});
        shell.helper(&Event::Topic { topic: "ai.prompt".into(), data: doc }, 1_200);
        assert_eq!(shell.state()["ai"]["up"], true);
        let mut acts = Vec::new();
        for (i, name) in ["screenremote", "search", "2", "ok"].into_iter().enumerate() {
            acts.extend(key(&mut shell, name, 2_000 + i as u64 * 100));
        }
        acts.extend(shell.key(&KeyEvent::Key { name: "voice".into(), down: true, repeat: false, seat: 0 }, 2_500));
        acts.extend(shell.key(&KeyEvent::Key { name: "voice".into(), down: false, repeat: false, seat: 0 }, 2_900));
        assert!(shell.state()["extras"]["remote"].is_null() && shell.state()["extras"]["voice"] == "" && shell.state()["home"]["layer"] == "", "{}", shell.state()["extras"]);
        assert!(luna_calls(&acts, "/launch").is_empty() && helper_calls(&acts, "/v1/voice/listen") == 0, "no app, no microphone: {acts:?}");
        assert_eq!(shell.state()["ai"]["up"], true, "and nothing answered the prompt");
    }

    #[test]
    fn with_the_home_kind_off_nothing_of_the_extras_is_reported_or_drawn() {
        let (_h, shell) = home_shell("osd");
        assert!(shell.state().get("extras").is_none() && shell.geometry().get("extras").is_none());
    }

    fn claims(acts: &[Act]) -> Vec<Value> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Channel(o) if o.path == "/v1/vwin" => o.body.clone(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_live_full_view_claims_a_video_window_once_and_lets_it_go_when_the_view_closes() {
        let (_h, mut shell) = shell();
        let camera = json!({"id": "273", "name": "Front Door", "room": "", "online": true, "live": true, "recordings": true, "events": true, "clips": false, "since": 0,
                            "native": {"codec": "h264", "width": 1920, "height": 1080, "fps": 15}, "prebuffered": true});
        shell.helper(&Event::Topic { topic: "cameras".into(), data: json!({"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1, "cameras": [camera]}) }, 1_000);
        assert!(claims(&shell.tick(1_050)).is_empty(), "nothing is open: nothing is asked for");
        assert!(shell.state().get("vwin").is_none());

        let mut acts = Vec::new();
        for e in shell.cameras().app.open_full("273", 1_100) {
            shell.carry(e, 1_100, &mut acts);
        }
        acts.extend(shell.tick(1_150));
        let posted = claims(&acts);
        assert_eq!(posted.len(), 1, "one claim for the picture: {posted:?}");
        assert_eq!((posted[0]["op"].as_str(), posted[0]["kind"].as_str(), posted[0]["id"].as_str()), (Some("claim"), Some("camera-full"), Some("cameras-full")));
        assert_eq!(posted[0]["rect"], json!({"x": 0, "y": 0, "w": 1920, "h": 1080}));
        assert!(claims(&shell.tick(1_200)).is_empty(), "the same wish is not posted again");
        assert_eq!(shell.state()["vwin"]["claims"]["cameras-full"], json!({"state": "asked"}));

        // the compositor grants the sub window; the shell reads it from the topic
        let line = json!({"id": "cameras-full", "kind": "camera-full", "state": "granted", "reason": "", "text": "", "slot": "sub", "placed": true});
        shell.channel(&Event::Topic { topic: "vwin".into(), data: json!({"mode": "apps", "slots": 2, "free": 1, "apps": 0, "wall": false, "claims": [line]}) }, 1_300);
        assert_eq!(shell.state()["vwin"]["claims"]["cameras-full"], json!({"state": "granted", "slot": "sub", "placed": true}));

        // the view closes: the claim is released
        let mut acts = Vec::new();
        for e in shell.cameras().app.close(1_400) {
            shell.carry(e, 1_400, &mut acts);
        }
        acts.extend(shell.tick(1_450));
        let posted = claims(&acts);
        assert_eq!(posted.iter().map(|b| b["op"].as_str().unwrap_or("")).collect::<Vec<_>>(), vec!["release"], "{posted:?}");
    }
}
