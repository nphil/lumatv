//! The system layer as the Rust shell answers it: the kinds the compositor's `--shell-rust` list names (`osd`: the volume and mute
//! display from the `audio` topic and the helper's `receiver` topic; `toast`: LG's toast feed), their models fed from the
//! channel, the helper, the keys and the clock, their surfaces drawn with the kit over the compositor's windows (stack 1, from
//! `system_z`), and the report the compositor reads (`POST /v1/ui`: `can`, `wants`, `live`, `state`). No I/O here: the host
//! (`main.rs`) carries out the [`Act`]s and feeds the events back, so the whole of it runs under a test with nothing but a clock.

use crate::cameras::app::Config as CamerasConfig;
use crate::cameras::host::Cameras;
use crate::home::model::NOT_MODIFIED;
use crate::home::{Home, HomeSurface};
use crate::host::http::{Event, Outbound};
use crate::host::keys::KeyEvent;
use crate::kit::pointer::{Guard, Sample};
use crate::kit::screen::{Effect, Input, Model};
use crate::kit::{Engine, Kit};
use crate::settings::host::SettingsHost;
use crate::settings::row::ShellRequest;
use crate::system::alert::ALERT_TAG;
use crate::panels::Typed;
use crate::panels::layer::{Panels, Which};
use crate::system::layer::Layer;
use crate::system::badge::Formats;
use crate::system::badge_surface::{BadgeSurface, SAMPLE};
use crate::system::toast::{FEED_TAG, Toasts};
use crate::system::toast_surface::ToastSurface;
use crate::system::volume::Volume;
use crate::system::volume_surface::VolumeSurface;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The Settings page the Cameras grid's `Settings` button opens (a page of Home & look).
const CAMERAS_SETTINGS_PAGE: &str = "cameras";
/// How long Home stays calm after an overlay is gone (the longest fade-out or slide-out is 220 ms).
const OVERLAY_RELEASE_MS: u64 = 450;
/// The quiet after the shell starts in which the icons' masks are loaded (`Icons::warm`), how many at a time, and how long between the steps.
const ICON_WARM_AFTER_MS: u64 = 2_000;
const ICON_WARM_STEP: usize = 3;
const ICON_WARM_GAP_MS: u64 = 30;
/// The longest a quiet tick (one that drew nothing and asked nothing) leaves the report unbuilt.
const REPORT_HEARTBEAT_MS: u64 = 1000;
/// The least time between two drawings for a refusal: a scene at its ceiling refuses what each drawing asks.
const REFUSAL_REDRAW_GAP_MS: u64 = 1000;

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
    /// The display lab: test cards and one setting of the screen's hardware at a time, driven with the remote (opened from Settings > Picture).
    pub displaylab: bool,
    /// The channel guide: LG's Live TV guide rebuilt (the channels and programmes from LG's own guide service), opened where LG's opens.
    pub guide: bool,
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
                "displaylab" => k.displaylab = true,
                "guide" => k.guide = true,
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
            (self.displaylab, "displaylab"),
            (self.guide, "guide"),
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
    /// The format badge (the size, dynamic range and sound of the picture in front, as marks): under the `toast` kind.
    formats: Formats,
    badge_surface: BadgeSurface,
    /// The system layer's kinds that sit over everything: the alerts, the cards, the pop-up, the capture mark, the saver.
    sys: Layer,
    home: Home,
    /// Wallpaper waits for Home to come to the foreground before taking its keys.
    wallpaper_waiting: bool,
    /// The latest confirmed theme and the temporary Settings patch, if any.
    theme_base: Value,
    theme_preview: Option<Value>,
    /// Rebuild glass surfaces after the strength slider comes to rest.
    glass_restyle_at: Option<u64>,
    theme_default_scale: f32,
    /// The text of the layout file Settings has seen (Home's), to hand it the layout again when Home changes it.
    settings_layout: String,
    home_surface: HomeSurface,
    /// The Cameras app (the grid, the full view, the strip, the sheet, the options page): a kind of its own over the panels.
    cameras: Cameras,
    /// The display's spare video windows the screens' live pictures are asked for (`--vwin` of the compositor): the claims posted and what the topic answered.
    vwin: crate::host::vwin::Vwin,
    /// The app the Cameras full view took the front from (Home came forward for it): Back brings it back.
    cam_return: Option<String>,
    /// The app whose video the Cameras full view paused (it is shown beside it): Back plays it again.
    cam_paused: Option<String>,
    /// Settings, the modal kind over everything but the system layer.
    settings: SettingsHost,
    /// The display lab: over Settings (it is opened from there) and under the editor and the system layer; it has the keys while it is open.
    lab: crate::lab::host::LabHost,
    /// The channel guide: LG's Live TV guide in LumaTV's look, over the live picture and under the panels; it has the keys while it is open.
    guide: crate::guide::host::GuideHost,
    /// The panels: the quick panel, the input picker and the notification drawer.
    panels: Panels,
    /// The press of OK or Back that is being routed was taken by a card (the pop-up's or the overlay queue's): the screens under do not see it.
    cards_took: bool,
    /// The camera (id, name) whose pop-up opened the snooze sheet that is up: its pick snoozes that camera or them all.
    snoozing: Option<(String, String)>,
    /// Which buttons of the pointer the screens may hear ([`Guard`]).
    pointer_guard: Guard,
    /// A pointer press that woke the saver retains its release.
    saver_pointer_down: bool,
    /// The last report sent, so an unchanged one is not sent again.
    reported: Option<Value>,
    /// The last text logged for each surface whose apply failed, and the surfaces that failed in this pass: a failure that repeats every frame is logged once.
    apply_errs: Vec<(&'static str, String)>,
    apply_failed_now: Vec<&'static str>,
    live: bool,
    // Last successful draws: volume, toast, Home, cameras, Settings, display lab, format badge.
    interactive_surfaces: u8,
    // Volume, toast and the format badge keep voting through the fades started by their surfaces.
    interactive_fades: [Option<u64>; 3],
    redraw: bool,
    /// The refusals the surfaces have drawn since (`Scene::refusals`), and when a new one may next be drawn for.
    refusals_seen: u64,
    refusal_redraw_at: u64,
    /// Whether any overlay (Settings, the display lab, a panel) was up at the last look, and until when Home counts it as over it, in ms (`u64::MAX`
    /// while it is up): Home is let go [`OVERLAY_RELEASE_MS`] after the event that finds the overlay gone, past its fade-out or slide-out, which a
    /// release in the middle of would make a frame of the whole stack at.
    overlay_up: bool,
    overlay_until: u64,
    /// When Home is next let go, a tick of its own.
    release_at: Option<u64>,
    /// When the next icon masks are loaded.
    icons_warm_at: Option<u64>,
    helper_up: bool,
    started: bool,
    /// The data directory (`theme/theme.json` is written under it) and the module's (`icons/`, `art/`).
    data_dir: Option<PathBuf>,
    /// The keys whose press the entry sheet took (their release is the sheet's too).
    entry_keys: Vec<String>,
    /// The keys whose press the display lab took (their release is the lab's too: the lab may have closed on the press, and no screen under it
    /// is to see a release whose press it never saw).
    lab_keys: Vec<String>,
    guide_keys: Vec<String>,
    /// Waking presses and their repeats/releases belong to the saver, even after it has gone.
    saver_keys: Vec<(String, u32)>,
    /// The size and time of each polled file (the theme, the layout) as it was last read: a poll that finds them unchanged reads nothing.
    polled_files: HashMap<u64, (SystemTime, u64)>,
    /// The buttons and visibility of the last pointer sample: a sample that changes neither only moves the pointer.
    pointer_state: Option<(u32, bool)>,
    /// Set by a pointer sample that only moved: unless it drew or asked for something, the report is as it was.
    pointer_only: bool,
    #[cfg(test)]
    reports_built: u32,
    /// The tick that called `after` asked nothing and drew nothing: its report is the last one's unless something besides the models changed it.
    quiet_tick: bool,
    /// When the report was last built (the shell's ms).
    reported_at: u64,
}

/// How old a polled file's change time must be before the read may be skipped on its account: a file written again within the clock's granularity
/// keeps its time and, with the same size, would look unchanged.
const STABLE_AGE: Duration = Duration::from_secs(2);

/// The size and modification time of `path`, when the file is old enough for them to tell a rewrite ([`STABLE_AGE`]).
fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    (SystemTime::now().duration_since(modified).ok()? >= STABLE_AGE).then_some((modified, meta.len()))
}

impl Shell {
    /// `icons` is the module's `icons/audio` directory (the format badge's marks are in `icons/video` beside it); `theme` the merged theme and
    /// `layout` Home's layout, for the placements.
    pub fn new(mut kit: Kit, kinds: Kinds, system_z: i32, icons: &Path, theme: &Value, layout: &Value) -> std::io::Result<Self> {
        kit.marks = crate::kit::logo::load_all(&icons.with_file_name("video"), &crate::kit::logo::VIDEO_MARKS);
        let engine = Engine::new(&kit).map_err(|e| std::io::Error::other(format!("{e}")))?;
        let home = Home::new(theme.clone(), crate::home::grid::shipped(), layout.clone());
        Ok(Self {
            volume: Volume::default(),
            volume_surface: VolumeSurface::new(system_z + crate::kit::z::SYSTEM, icons, theme, layout),
            toasts: Toasts::new(),
            toast_surface: ToastSurface::new(system_z + crate::kit::z::SYSTEM + 10, theme, layout),
            formats: Formats::new(),
            badge_surface: BadgeSurface::new(system_z + crate::kit::z::SYSTEM + 14, theme, layout),
            sys: Layer::new(system_z + crate::kit::z::SYSTEM, theme, layout),
            // Settings reads the layout as Home has merged and tidied it
            settings: SettingsHost::new(system_z, theme, &home.layout),
            lab: crate::lab::host::LabHost::new(system_z + crate::kit::z::LAB),
            guide: crate::guide::host::GuideHost::new(system_z + crate::kit::z::GUIDE),
            settings_layout: home.layout_text.clone(),
            theme_base: theme.clone(),
            theme_default_scale: kit.text_scale,
            theme_preview: None,
            glass_restyle_at: None,
            home,
            wallpaper_waiting: false,
            home_surface: HomeSurface::new(system_z),
            cameras: Cameras::new(system_z + crate::kit::z::CAMERAS, CamerasConfig::from_theme(&kit, theme)),
            vwin: crate::host::vwin::Vwin::new(),
            cam_return: None,
            cam_paused: None,
            panels: Panels::new(system_z + crate::kit::z::PANELS, theme),
            kit,
            engine,
            kinds,
            cards_took: false,
            snoozing: None,
            pointer_guard: Guard::default(),
            saver_pointer_down: false,
            reported: None,
            apply_errs: Vec::new(),
            apply_failed_now: Vec::new(),
            live: false,
            interactive_surfaces: 0,
            interactive_fades: [None; 3],
            redraw: false,
            refusals_seen: 0,
            refusal_redraw_at: 0,
            overlay_up: false,
            overlay_until: 0,
            release_at: None,
            icons_warm_at: None,
            helper_up: false,
            started: false,
            data_dir: None,
            entry_keys: Vec::new(),
            lab_keys: Vec::new(),
            guide_keys: Vec::new(),
            saver_keys: Vec::new(),
            polled_files: HashMap::new(),
            pointer_state: None,
            pointer_only: false,
            #[cfg(test)]
            reports_built: 0,
            quiet_tick: false,
            reported_at: 0,
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
        self.icons_warm_at = Some(crate::host::now_ms() + ICON_WARM_AFTER_MS);
        self.home_surface.dirs(module, data);
        self.data_dir = Some(data.to_path_buf());
    }

    /// What goes out at the start: the feeds this process subscribes to, and the first report.
    pub fn start(&mut self) -> Vec<Act> {
        self.started = true;
        let mut acts = Vec::new();
        if self.kinds.toast {
            let effects = [self.toasts.subscribe(), self.formats.subscribe()].concat();
            for e in effects {
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
        acts.push(Act::Channel(Outbound { method: "POST", path: "/v1/ui".into(), body: Some(self.report()), tag: None, beside: false }));
        acts
    }

    /// Logs a surface's failed apply when its text differs from the one logged last; the apply runs every frame and a failure that stays
    /// (`settings: at most 96 groups`) filled the log at the frame rate.
    fn apply_failed(&mut self, src: &'static str, text: String, acts: &mut Vec<Act>) {
        self.apply_failed_now.push(src);
        match self.apply_errs.iter_mut().find(|(s, _)| *s == src) {
            Some((_, last)) if *last == text => {}
            Some((_, last)) => {
                *last = text.clone();
                acts.push(Act::Log(text));
            }
            None => {
                self.apply_errs.push((src, text.clone()));
                acts.push(Act::Log(text));
            }
        }
    }

    fn carry(&mut self, effect: Effect, now: u64, acts: &mut Vec<Act>) {
        match effect {
            Effect::Luna { tag, uri, payload, subscribe } => acts.push(Act::Channel(Outbound {
                method: "POST",
                path: "/v1/luna".into(),
                body: Some(json!({"tag": tag, "uri": uri, "payload": payload, "subscribe": subscribe})),
                tag: None,
                beside: false,
            })),
            Effect::Cancel { tag } => {
                acts.push(Act::Channel(Outbound { method: "POST", path: "/v1/cancel".into(), body: Some(json!({"tag": tag})), tag: None, beside: false }))
            }
            Effect::Post { path, body } => acts.push(Act::Helper(Outbound { method: "POST", path, body: Some(body), tag: None, beside: false })),
            Effect::Fetch { tag, path } => acts.push(Act::Helper(Outbound { method: "GET", path, body: None, tag: Some(tag), beside: true })),
            Effect::Request { tag, method, path, body } => {
                let method = match method.as_str() {
                    "GET" => "GET",
                    "PUT" => "PUT",
                    "DELETE" => "DELETE",
                    _ => "POST",
                };
                acts.push(Act::Helper(Outbound { method, path, body, tag: Some(tag), beside: false }))
            }
            Effect::Channel { path, body } => acts.push(Act::Channel(Outbound { method: "POST", path, body: Some(body), tag: None, beside: false })),
            Effect::Theme { tag, patch } => self.write_theme(tag, &patch, now, acts),
            Effect::Read { tag, path } => {
                let full = self.file_of(&path);
                // the theme and the layout are polled every 2 s: the read, the parse and the comparison are skipped while the file is as it was.
                // The layout is not skipped while its editor is open (Home leaves what it reads then unused, to be read again when it closes).
                let watched = tag == crate::home::model::THEME_TAG || tag == crate::home::model::LAYOUT_TAG;
                let polled = watched && !(tag == crate::home::model::LAYOUT_TAG && self.home.editor.is_some());
                let stamp = full.as_ref().ok().filter(|_| polled).and_then(|f| file_stamp(f));
                if stamp.is_some() && self.polled_files.get(&tag) == stamp.as_ref() {
                    let input = Input::Fetched { tag, status: NOT_MODIFIED, data: Value::Null };
                    for e in self.home.update(&input, now) {
                        self.carry(e, now, acts);
                    }
                    return;
                }
                let (status, data) = match full.map(std::fs::read) {
                    Ok(Ok(bytes)) => match serde_json::from_slice::<Value>(&bytes) {
                        Ok(v) => (200, v),
                        Err(e) => (500, json!({"error": e.to_string()})),
                    },
                    Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => (404, Value::Null),
                    Ok(Err(e)) => (500, json!({"error": e.to_string()})),
                    Err(e) => (500, json!({"error": e.to_string()})),
                };
                if watched {
                    match stamp {
                        Some(stamp) if status == 200 => self.polled_files.insert(tag, stamp),
                        _ => self.polled_files.remove(&tag),
                    };
                }
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
        if status == 200 {
            self.theme_base = data.clone();
            self.home.theme_text = data.to_string();
            let tokens = lumatv_tokens::Tokens::from_json(&self.theme_base);
            let mut previous = self.kit.tokens.json().clone();
            previous["glassStrength"] = tokens.json()["glassStrength"].clone();
            if patch.as_object().is_some_and(|p| p.len() == 1 && p.contains_key("glassStrength"))
                && self.theme_preview.is_none() && previous == *tokens.json()
            {
                // Only the panes need the live strength; the other surfaces can wait until the slider rests.
                self.settings.set_theme(tokens.json().clone());
                self.kit.tokens = tokens;
                self.settings.restyle_glass(&self.kit);
                self.glass_restyle_at = Some(now + 350);
            } else {
                self.glass_restyle_at = None;
                self.apply_theme_look(now, acts, true);
            }
        }
        self.feed(&Input::Fetched { tag, status, data }, now, acts);
    }

    fn feed(&mut self, input: &Input, now: u64, acts: &mut Vec<Act>) {
        if let Input::Fetched { tag, status: 200, data } = input
            && *tag == crate::home::model::THEME_TAG
            && *data != self.theme_base
        {
            self.theme_base = data.clone();
            self.glass_restyle_at = None;
            self.apply_theme_look(now, acts, true);
        }
        // the compositor's `refresh` topic: the Hz the output runs at (`ctl refresh 120`)
        if let Input::Topic { topic, data } = input
            && topic == "refresh"
            && let Some(hz) = data.get("hz").and_then(Value::as_f64)
        {
            if self.kit.refresh_hz < crate::kit::motion::INSTANT_MODAL_HZ && hz as f32 >= crate::kit::motion::INSTANT_MODAL_HZ {
                self.redraw = true;
            }
            self.kit.refresh_hz = hz as f32;
            // The compositor also replays this topic after attaching the engine.
            self.reported = None;
        }
        // Who had the keys when this input arrived: a key that answers an alert or wakes the saver was theirs, and the screens under them
        // never see it (the key that wakes the TV does nothing else).
        let key_like = matches!(input, Input::Key { .. } | Input::Pointer { .. });
        let settings_had = self.kinds.settings && self.settings.is_open();
        let cards_had = matches!(input, Input::Key { .. }) && self.cards_took;
        let lab_had = key_like && self.lab_wants();
        let layer_had = key_like && (self.sys.wants(&self.kinds) || cards_had || lab_had);
        let system_had = key_like && (self.system_wants() || cards_had || lab_had);
        if self.kinds.osd {
            for e in self.volume.update(input, now) {
                self.carry(e, now, acts);
            }
        }
        if self.kinds.toast {
            let effects = [self.toasts.update(input, now), self.formats.update(input, now)].concat();
            for e in effects {
                self.carry(e, now, acts);
            }
        }
        for e in self.sys.update(&self.kinds, input, now) {
            self.carry(e, now, acts);
        }
        // the lab hears the topics, the helper's answers and the clock here; its keys and its pointer come by `key`
        if self.kinds.displaylab && !key_like {
            for e in self.lab.update(input, now) {
                self.carry(e, now, acts);
            }
        }
        // the channel guide hears the topics (what is in front), the luna answers and the clock here
        if self.kinds.guide && !key_like {
            for e in self.guide.update(input, now) {
                self.carry(e, now, acts);
            }
        }
        for e in self.panels.update(&self.kinds, input, now) {
            self.carry(e, now, acts);
        }
        self.panel_events(now, acts);
        if self.kinds.home {
            // a kind above Home that owns input takes the keys and the pointer before Home sees them (`system_wants` is the one order of owners)
            if system_had {
                self.home.input_taken(input);
            } else {
                for e in self.home.update(input, now) {
                    self.carry(e, now, acts);
                }
                for r in self.home.take_requests() {
                    // the Settings and the drawer are other kinds' surfaces: routed to them when they are in the list
                    match r {
                        crate::home::model::Request::Quick => self.open_panel_or_app(Which::Quick, self.kinds.quick, "com.palm.app.settings", now, acts),
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
                        crate::home::model::Request::Cameras if self.kinds.cameras => {
                            for e in self.cameras.open_grid(now) {
                                self.carry(e, now, acts);
                            }
                            self.cameras_asks(now, acts);
                            self.redraw = true;
                        }
                        crate::home::model::Request::Cameras => acts.push(Act::Log("home asks for cameras".into())),
                        // a camera item of the smart-home card: the viewer at full size, opened on the picture the card shows of it
                        crate::home::model::Request::CameraFull(id) if self.kinds.cameras => {
                            let pic = self.home.stills.picture(&id).cloned();
                            self.open_camera_viewer(&id, pic, now, acts);
                        }
                        crate::home::model::Request::CameraFull(_) => acts.push(Act::Log("home asks for a camera".into())),
                        // the camera button: the list of every camera grows out of it
                        crate::home::model::Request::CameraList if self.kinds.cameras => {
                            let mut out = Vec::new();
                            self.panels.show(Which::Cameras, false, &self.kinds, now, &mut out);
                            for e in out {
                                self.carry(e, now, acts);
                            }
                        }
                        crate::home::model::Request::CameraList => acts.push(Act::Log("home asks for the camera list".into())),
                    }
                }
            }
        }
        if self.wallpaper_waiting {
            if matches!(input, Input::Key { down: true, .. } | Input::Pointer { buttons: 1.., .. }) || !self.settings.is_open() {
                self.wallpaper_waiting = false;
            } else if self.home.active {
                self.wallpaper_waiting = false;
                self.open_wallpaper(now, acts);
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
        // Opening Settings from Home does not hand the same press to its first row.
        if self.kinds.settings && !layer_had && (!key_like || settings_had) {
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

    /// Whether a kind that sits above Home owns input now: an alert, the input popup, a PIN prompt, the saver, the layout editor, Home's own
    /// extras, a panel (quick, picker, drawer, menu, confirm, switcher, the entry sheet), the apps' keyboard, the Cameras viewer, Settings or the display lab.
    /// Home gets no key and no pointer sample while one does; the volume display and the toasts never own input. Every gate that keeps Home
    /// out of input asks this, so a new owner is added here and nowhere else.
    pub fn system_wants(&self) -> bool {
        self.sys.wants(&self.kinds)
            || self.panels.wants(&self.kinds)
            || self.panels.keyboard_up(&self.kinds)
            || (self.kinds.cameras && self.cameras.wants())
            || (self.kinds.settings && self.settings.is_open())
            || self.lab_wants()
            || self.guide_wants()
    }

    /// The display lab is open and has the keys and the pointer: nothing under it hears them.
    fn lab_wants(&self) -> bool {
        self.kinds.displaylab && self.lab.wants()
    }

    /// The channel guide is open and has the keys and the pointer: nothing under it hears them but a panel opened over it.
    fn guide_wants(&self) -> bool {
        self.kinds.guide && self.guide.wants()
    }

    /// The host harness answers the guide's calls of LG's services itself, with a make-believe set of channels (`--guide-fixture`).
    pub fn guide_fixture(&mut self, logos: Option<&std::path::Path>) {
        self.guide.with_fixture(logos);
    }

    /// What the panels tell the rest of the shell goes to every model as a topic; "All settings" opens Settings at its page.
    fn panel_events(&mut self, now: u64, acts: &mut Vec<Act>) {
        for (topic, data) in self.panels.take_events() {
            if topic == "settings.open" {
                let page = data.get("page").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string);
                acts.extend(self.open_settings(page.as_deref(), now));
            }
            // a row of the camera list: the viewer opens on that camera, on the picture the row shows (the list has closed itself)
            if topic == "cameralist.open"
                && self.kinds.cameras
                && let Some(id) = data.get("camera").and_then(Value::as_str)
            {
                let pic = self.panels.camera_picture(id);
                self.open_camera_viewer(id, pic, now, acts);
            }
            // the snooze sheet a held OK on the camera pop-up opened: the snooze is asked for, the card it was for ends, and a toast says so
            if topic == "menu.picked"
                && data.get("id").and_then(Value::as_str) == Some(crate::dnd::SNOOZE_FORM)
                && let Some((camera, name)) = self.snoozing.take()
            {
                let (source, label, time) = crate::dnd::snooze_picked(&camera, &name, data.get("values").unwrap_or(&Value::Null));
                self.carry(Effect::Post { path: "/v1/dnd/snooze".into(), body: crate::dnd::snooze_body(crate::dnd::CAMERA, source.as_deref(), &label, time) }, now, acts);
                for e in self.sys.popup_snoozed(&self.kinds, source.as_deref()) {
                    self.carry(e, now, acts);
                }
                let toast = json!({"sourceId": "com.webos.app.home", "message": crate::dnd::snoozed_words(&label, time), "noaction": true, "persistent": false});
                self.carry(Effect::Luna { tag: crate::system::layer::TOAST_TAG, uri: "luna://com.webos.notification/createToast".into(), payload: toast, subscribe: false }, now, acts);
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

    /// The Cameras viewer at full size on camera `id`, opened on `picture` (what a card or the list shows of it) until its own pictures come.
    fn open_camera_viewer(&mut self, id: &str, picture: Option<crate::kit::picture::Picture>, now: u64, acts: &mut Vec<Act>) {
        if let Some(pic) = picture {
            self.cameras.seed_full(id, pic);
        }
        for e in self.cameras.open_full(id, now) {
            self.carry(e, now, acts);
        }
        self.cameras_asks(now, acts);
        self.redraw = true;
    }

    /// Opens Settings, at `page` when one is named: a tile of the quick panel, a card of Home, the compositor's `show`.
    pub fn open_settings(&mut self, page: Option<&str>, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        if self.kinds.settings {
            self.settings_open(page, now, &mut acts);
            self.after(now, &mut acts);
        }
        acts
    }

    /// Settings comes up over a panel the compositor showed, and that panel closes.
    fn settings_open(&mut self, page: Option<&str>, now: u64, acts: &mut Vec<Act>) {
        if !self.settings.is_open() {
            let mut out = Vec::new();
            self.panels.close_shown(&self.kinds, now, &mut out);
            for e in out {
                self.carry(e, now, acts);
            }
        }
        for e in self.settings.open(page, now) {
            self.carry(e, now, acts);
        }
        self.redraw = true;
    }

    /// Opens the overlay layout editor on `item` (a registry id: `volume`, `mute`, `popup`, `cards`, `toast`, `format`; the first when it is none) over
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
        sizes.set("toast", lumatv_placement::Size::new(320, self.toast_surface.sample_height(&self.kit).ceil() as i32));
        sizes.set("format", BadgeSurface::sample_size(&self.kit));
        sizes.set("mute", lumatv_placement::Size::new(64, 64));
        // the box of the status bar's buttons as Home lays it out now: the profile chip's name and the text size make it as wide as it is (the glass
        // behind it may be the whole row, which is the placement's to say)
        let group = self.home.status_group();
        sizes.set("statusbar", lumatv_placement::Size::new(group.w.round() as i32, group.h.round() as i32));
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
            Event::Up(false) => self.feed(&Input::Topic { topic: "channel.up".into(), data: json!(false) }, now, &mut acts),
            Event::Up(true) => {},
            Event::State(topics) => {
                self.feed(&Input::Topic { topic: "channel.up".into(), data: json!(false) }, now, &mut acts);
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
                self.settings_open(data.get("page").and_then(Value::as_str), now, &mut acts);
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("layout") && self.kinds.layout => {
                // `ctl shell show layout ITEM` names the item as the page
                self.start_layout(data.get("item").or_else(|| data.get("page")).and_then(Value::as_str));
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("displaylab") && self.kinds.displaylab => {
                // `ctl shell show displaylab`: the same door as Settings' row
                for e in self.lab.open(now) {
                    self.carry(e, now, &mut acts);
                }
                self.redraw = true;
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("guide") && self.kinds.guide => {
                // `ctl shell show guide`: the same door as the Guide key
                for e in self.guide.open(now) {
                    self.carry(e, now, &mut acts);
                }
                self.redraw = true;
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("banner") && self.kinds.banner => {
                // `ctl shell show banner pill|bar|nosignal`: what the QML's development door was, for a TV check
                for e in self.sys.show_banner(data.get("page").and_then(Value::as_str).unwrap_or("pill"), now) {
                    self.carry(e, now, &mut acts);
                }
                self.redraw = true;
            }
            Event::Topic { topic, data } if topic == "show" && data.get("kind").and_then(Value::as_str) == Some("toast") && self.kinds.toast => {
                // `ctl shell show toast [TEXT]`: a toast through the feed's own path, to check where toasts sit without LG's createToast. In TEXT an
                // underscore is a space and a bar a line break (the first line is the title, the rest the body); it stays up 8 s.
                let page = data.get("page").and_then(Value::as_str).filter(|t| !t.is_empty());
                if let Some(names) = page.and_then(|p| if p == "format" { Some("") } else { p.strip_prefix("format:") }) {
                    // `format[:MARKS]`: the format badge with these marks (comma separated, `kit::logo::VIDEO_MARKS`), a 4K Dolby Vision film with
                    // Atmos when none are named
                    let marks: Vec<&'static str> = names.split(',').filter_map(|n| crate::kit::logo::VIDEO_MARKS.into_iter().find(|m| *m == n.trim())).collect();
                    self.formats.show_marks(if marks.is_empty() { SAMPLE.to_vec() } else { marks }, now);
                } else {
                    let text = page.unwrap_or("Test_toast|Raised_by_ctl_shell_show_toast");
                    let payload = json!({"sourceId": "lumatv", "message": text.replace('_', " ").replace('|', "\n"), "timeout": 8});
                    self.feed(&Input::Luna { tag: FEED_TAG, payload }, now, &mut acts);
                }
                self.redraw = true;
            }
            Event::Topic { topic, data } => {
                self.feed(&Input::Topic { topic: topic.clone(), data: data.clone() }, now, &mut acts);
            }
            // only the helper's feed holds its events back (`Feed::expand`)
            Event::Answer { .. } | Event::Bytes { .. } | Event::Waiting => {}
        }
        self.after(now, &mut acts);
        acts
    }

    /// An event of the helper's API.
    pub fn helper(&mut self, event: &Event, now: u64) -> Vec<Act> {
        self.helper_all(std::slice::from_ref(event), now)
    }

    /// Events of the helper's API that came together: taken in order and drawn once, at the end. A page being made asks for five or six answers, and
    /// each drew Home, the panels and Settings again and built the report: 0.3-1.5 ms of the shell's thread apiece on the host.
    pub fn helper_all(&mut self, events: &[Event], now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        for event in events {
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
                // the loop hands its feed's events over expanded (`Feed::expand`)
                Event::Waiting => {}
            }
        }
        self.after(now, &mut acts);
        acts
    }

    /// A key or a pointer sample from the key socket: the layer's alerts take the keys while one is up, the pointer's hover and click
    /// are the kit's rule.
    pub fn key(&mut self, event: &KeyEvent, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        if let KeyEvent::Key { name, down, repeat, seat } = event {
            if let Some(i) = self.saver_keys.iter().position(|(key, owner)| key == name && owner == seat) {
                if !*down {
                    self.saver_keys.remove(i);
                }
                self.after(now, &mut acts);
                return acts;
            }
            if *down && self.sys.saver_up(&self.kinds) && !self.sys.questions_up(&self.kinds) {
                self.saver_keys.push((name.clone(), *seat));
                for e in self.sys.update(&self.kinds, &Input::Key { name: name.clone(), down: *down, repeat: *repeat }, now) {
                    self.carry(e, now, &mut acts);
                }
                self.after(now, &mut acts);
                return acts;
            }
        }
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
        // the Guide key: claimed from Live TV (the compositor gives it over only while the guide lists it), so LG's guide never launches. It opens the
        // guide unless something else has the screen; nothing else hears it
        if let KeyEvent::Key { name, down, repeat, .. } = event
            && name == "guide"
        {
            if self.kinds.guide && *down && !*repeat && !self.system_wants() {
                for e in self.guide.guide_key(now) {
                    self.carry(e, now, &mut acts);
                }
                self.redraw = true;
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
                // the display lab has every key while it is open (a question of the system layer or a card first), and the release of a key it took:
                // the lab may have closed on the press, and what is under it is to see neither
                let lab_up = self.lab_wants() && !self.sys.wants(&self.kinds) && !self.cards_took;
                let lab_took = self.lab_keys.iter().position(|k| k == name);
                let for_lab = lab_up || lab_took.is_some();
                if *down && lab_up && lab_took.is_none() {
                    self.lab_keys.push(name.clone());
                }
                if !*down && let Some(i) = lab_took {
                    self.lab_keys.remove(i);
                }
                // the channel guide has every key while it is open, unless a card, a question, Settings or a panel opened over it has them; and the
                // release of a key it took (it may have closed on the press, and what is under it is to see neither)
                let guide_up = self.guide_wants() && !self.sys.wants(&self.kinds) && !self.cards_took && !for_lab && !self.panels.wants(&self.kinds) && !self.settings.is_open();
                let guide_took = self.guide_keys.iter().position(|k| k == name);
                let for_guide = guide_up || guide_took.is_some();
                if *down && guide_up && guide_took.is_none() {
                    self.guide_keys.push(name.clone());
                }
                if !*down && let Some(i) = guide_took {
                    self.guide_keys.remove(i);
                }
                if for_lab {
                    for e in self.lab.key(name, *down, *repeat, &mut self.engine, now) {
                        self.carry(e, now, &mut acts);
                    }
                } else if for_guide {
                    for e in self.guide.key(name, *down, *repeat, &mut self.engine, now) {
                        self.carry(e, now, &mut acts);
                    }
                } else if sys_asks {
                    for e in self.sys.update(&self.kinds, &Input::Key { name: name.clone(), down: *down, repeat: *repeat }, now) {
                        self.carry(e, now, &mut acts);
                    }
                } else if !for_entry {
                    self.feed(&Input::Key { name: name.clone(), down: *down, repeat: *repeat }, now, &mut acts);
                }
                // a panel has the keys unless a question of the system layer or Settings is over it, or a card took the key (the entry sheet is over Settings)
                if !self.sys.wants(&self.kinds) && (for_entry || !self.settings.is_open()) && !self.cards_took && !for_lab && !for_guide {
                    for e in self.panels.key(&self.kinds, name, *down, *repeat, now) {
                        self.carry(e, now, &mut acts);
                    }
                    self.panel_events(now, &mut acts);
                }
            }
            KeyEvent::Pointer { x, y, buttons, visible } => {
                self.cards_took = false;
                let now_ns = self.kit.ui.scene.now_ns();
                // no screen hears a button that was not seen going down, or a button on a hidden pointer
                let sample = self.pointer_guard.filter(Sample { x: *x, y: *y, buttons: *buttons, visible: *visible });
                let steady = self.pointer_state.replace((sample.buttons, sample.visible)) == Some((sample.buttons, sample.visible));
                if self.saver_pointer_down {
                    if !sample.visible || sample.buttons == 0 {
                        self.saver_pointer_down = false;
                    }
                    self.after(now, &mut acts);
                    return acts;
                }
                if self.sys.saver_up(&self.kinds) && !self.sys.questions_up(&self.kinds) {
                    self.saver_pointer_down = sample.visible && sample.buttons != 0;
                    for e in self.sys.update(&self.kinds, &Input::Pointer { x: sample.x, y: sample.y, buttons: sample.buttons, visible: sample.visible }, now) {
                        self.carry(e, now, &mut acts);
                    }
                    self.after(now, &mut acts);
                    return acts;
                }
                let (buttons, visible) = (&sample.buttons, &sample.visible);
                // the layout editor, the AI prompt and the display lab are the whole screen's: nothing under them hears the pointer
                let lab_up = self.lab_wants();
                // the guide is the whole screen's too, but a panel opened over it (the quick panel) and Settings come first
                let guide_up = self.guide_wants() && !lab_up && !self.panels.wants(&self.kinds) && !(self.kinds.settings && self.settings.is_open());
                let editor = self.sys.layout_open() || self.sys.ai_up() || lab_up || guide_up;
                if let Some(target) = self.sys.pointer(&self.kit, &mut self.engine, now_ns, now, sample) {
                    for e in self.sys.press(target) {
                        self.carry(e, now, &mut acts);
                    }
                } else if lab_up {
                    for e in self.lab.pointer(&self.kit, &mut self.engine, now_ns, sample, now) {
                        self.carry(e, now, &mut acts);
                    }
                } else if guide_up {
                    for e in self.guide.pointer(&self.kit, &mut self.engine, now_ns, sample, now) {
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
                if !editor {
                    for e in self.panels.pointer(&self.kinds, &self.kit, &mut self.engine, now_ns, sample, now) {
                        self.carry(e, now, &mut acts);
                    }
                }
                self.panel_events(now, &mut acts);
                self.pointer_only = steady;
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
        if self.icons_warm_at.is_some_and(|at| now >= at) {
            self.icons_warm_at = self.kit.icons.warm(ICON_WARM_STEP).then_some(now + ICON_WARM_GAP_MS);
        }
        if self.glass_restyle_at.is_some_and(|at| now >= at) {
            self.apply_theme_look(now, &mut acts, false);
            self.glass_restyle_at = None;
        }
        // the pictures of the inputs are taken in the quiet: nothing of the shell is up
        let quiet = !(self.panels.wants(&self.kinds)
            || (self.kinds.settings && self.settings.is_open())
            || (self.kinds.displaylab && self.lab.is_open())
            || (self.kinds.guide && self.guide.is_open())
            || (self.kinds.cameras && self.cameras.wants())
            || self.sys.saver_up(&self.kinds));
        self.panels.set_quiet(quiet);
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
        // Home behind an app has nothing on the screen to keep up to date: its clock's second hand and its news slide each cost the compositor a
        // composed frame a second under a playing video (the hole wipes what they drew), so they wait for Home to be in front again.
        if self.kinds.home && self.home.active {
            if self.home_surface.tick(now) {
                self.redraw = true;
            } else if self.home_surface.art_due(now) && !self.redraw {
                self.home_surface.step_art(&self.kit, &self.home, now);
            }
        }
        if self.kinds.cameras && self.cameras.surface.tick(now) {
            self.redraw = true;
        }
        if self.kinds.settings && self.settings.tick(&mut self.engine, now) {
            self.redraw = true;
        }
        self.quiet_tick = !self.redraw && acts.is_empty();
        self.after(now, &mut acts);
        acts
    }

    /// Draws what changed and reports when the report changed.
    fn after(&mut self, now: u64, acts: &mut Vec<Act>) {
        for fade in &mut self.interactive_fades {
            if fade.is_some_and(|until| now >= until) { *fade = None; }
        }
        // the material in use and the saver: what the moving wallpaper's clip needs to be Home's besides the theme
        if self.kinds.home {
            let (light, saver) = (self.kit.mode == ui::lucent::palette::Mode::Light, self.sys.saver_up(&self.kinds));
            for e in self.home.set_context(light, saver, now) {
                self.carry(e, now, acts);
            }
        }
        // what the panels, Settings and the Cameras viewer have up is what the cards and the pop-up wait for
        let modal = (self.kinds.settings && self.settings.is_open()) || (self.kinds.displaylab && self.lab.is_open()) || (self.kinds.guide && self.guide.is_open());
        if self.kinds.home {
            self.home.set_covered(modal);
        }
        let host_busy = self.panels.wants(&self.kinds) || modal;
        // Home is calm (its wallpaper stands still and its weather rests) while something is over it, until the overlay has been gone a while (not
        // at the start of its fade-out)
        if host_busy != self.overlay_up {
            self.overlay_up = host_busy;
            self.overlay_until = if host_busy { u64::MAX } else { now + OVERLAY_RELEASE_MS };
        }
        if self.kinds.home && self.home.set_calm(now < self.overlay_until) {
            self.redraw = true;
        }
        // a tick of its own when it ends
        self.release_at = (self.overlay_until > now && self.overlay_until != u64::MAX).then_some(self.overlay_until);
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
        if self.kinds.displaylab && self.lab.take_redraw() {
            self.redraw = true;
        }
        if self.kinds.guide && self.guide.take_redraw() {
            self.redraw = true;
        }
        // what the compositor made of the live picture's claim on a video window: the full view follows it
        if self.kinds.cameras {
            self.cameras.app.set_vwin_on(self.vwin.on());
            self.cameras.app.set_app_video(self.vwin.app_playing());
            let id = crate::cameras::app::CLAIM_ID;
            let mut window = crate::cameras::app::Window::of(self.vwin.grant(id));
            // a window that draws before videooutputd has the video on a sink is a black box: the view keeps its picture until it has
            if window == crate::cameras::app::Window::Carried && self.vwin.connected(id) == Some(false) {
                window = crate::cameras::app::Window::Waiting;
            }
            if self.cameras.app.set_window(window) {
                self.redraw = true;
            }
            // a full view beside an app's video asks the helper for its stream once its window is granted
            for e in self.cameras.app.window_answered(now) {
                self.carry(e, now, acts);
            }
        }
        // ... and the camera pop-up's: its stream is asked for once a window is granted, and taken away when the window is
        if self.kinds.popup {
            let id = crate::system::popup_live::CLAIM_ID;
            let window = crate::cameras::app::Window::of(self.vwin.grant(id));
            let slot = match self.vwin.grant(id) {
                crate::host::vwin::Grant::Granted { slot, .. } => slot.clone(),
                // why it was refused: `both-scalers` (an app holds both) keeps the stills quiet
                crate::host::vwin::Grant::Denied { reason, .. } | crate::host::vwin::Grant::Preempted { reason, .. } => reason.clone(),
                _ => String::new(),
            };
            let sink = self.vwin.sink(id).is_some();
            for e in self.sys.popup_window(&self.kinds, window, &slot, sink, now) {
                self.carry(e, now, acts);
            }
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
        // a refusal may have left a part a surface keeps unmade: drawing makes it again (`kit::mend`)
        let refusals = self.kit.ui.scene.refusals();
        if refusals != self.refusals_seen && now >= self.refusal_redraw_at {
            self.refusals_seen = refusals;
            self.refusal_redraw_at = now + REFUSAL_REDRAW_GAP_MS;
            self.redraw = true;
        }
        // a pointer that only moved, and drew nothing and asked for nothing, leaves the report as it was
        let pointer_only = std::mem::take(&mut self.pointer_only) && !self.redraw && acts.is_empty();
        let redrew = self.redraw;
        if self.redraw {
            self.redraw = false;
            let mut live = false;
            let mut interactive = self.interactive_surfaces;
            let set_visible = |bits: &mut u8, bit: u8, on: bool| {
                if on { *bits |= bit; } else { *bits &= !bit; }
            };
            if self.kinds.osd {
                // a mute mark the owner placed drifts about its place, clear of the cards and the panel that are up
                let mut avoid = self.sys.avoid(&self.kinds);
                let panel = self.panels.avoid(&self.kinds);
                // a panel up on the right (its title is where the mark likes to be) keeps the mark's own corners to the left, as the pop-up does
                self.volume.avoid_top_right = !panel.is_empty();
                avoid.extend(panel);
                self.volume.set_placed(self.volume_surface.mark_places(&avoid));
                let view = self.volume.view();
                match self.volume_surface.apply(&mut self.kit, &view, now) {
                    Ok(on) => {
                        live |= on;
                        if view.up { self.interactive_fades[0] = None; }
                        else if interactive & 1 != 0 {
                            let ms = u64::from(self.kit.beat_ms(crate::kit::motion::Beat::Rest));
                            self.interactive_fades[0] = (ms != 0).then_some(now.saturating_add(ms));
                        }
                        set_visible(&mut interactive, 1, view.up);
                    },
                    Err(e) => {
                        if self.volume_surface.pill_visible() { interactive |= 1; }
                        acts.push(Act::Log(format!("volume: {e}")));
                    },
                }
            }
            if self.kinds.toast {
                let view = self.toasts.view();
                match self.toast_surface.apply(&mut self.kit, &view, self.volume_surface.pane_rect(), self.sys.app_front()) {
                    Ok(on) => {
                        live |= on;
                        if on { self.interactive_fades[1] = None; }
                        else if interactive & 2 != 0 {
                            let ms = u64::from(self.kit.beat_ms(crate::kit::motion::Beat::Layer));
                            self.interactive_fades[1] = (ms != 0).then_some(now.saturating_add(ms));
                        }
                        set_visible(&mut interactive, 2, on);
                    },
                    Err(e) => self.apply_failed("toasts", format!("toasts: {e}"), acts),
                }
            }
            match self.sys.apply(&self.kinds, &mut self.kit, &mut self.engine, now) {
                Ok(on) => live |= on,
                Err(e) => self.apply_failed("system", e.to_string(), acts),
            }
            if self.kinds.toast {
                // after the system layer has placed this frame's pop-up and cards: the badge steps aside from them, the volume bar and a toast
                let mut up = self.sys.avoid(&self.kinds);
                up.extend(self.volume_surface.rects());
                up.extend(self.toast_surface.rects().iter().map(|r| lumatv_placement::Rect::new(r.x as i32, r.y as i32, r.w.ceil() as i32, r.h.ceil() as i32)));
                match self.badge_surface.apply(&mut self.kit, self.formats.view().as_ref(), &up, self.sys.app_front()) {
                    Ok(on) => {
                        live |= on;
                        if on { self.interactive_fades[2] = None; }
                        else if interactive & 128 != 0 {
                            let ms = u64::from(self.kit.beat_ms(crate::kit::motion::Beat::Layer));
                            self.interactive_fades[2] = (ms != 0).then_some(now.saturating_add(ms));
                        }
                        set_visible(&mut interactive, 128, on);
                    },
                    Err(e) => self.apply_failed("format", format!("format badge: {e}"), acts),
                }
            }
            // the pop-up's picture box is known once the card is drawn: its claim on a video window is made now, not with the next input
            if self.kinds.popup {
                for e in self.vwin.want(&self.vwin_wants(), now) {
                    self.carry(e, now, acts);
                }
            }
            let now_ns = self.kit.ui.scene.now_ns();
            // Settings is the modal kind over everything and needs the room (the scene has 96 groups in all, Home holds 64 of them): a panel
            // that is only kept built for its next opening goes at once, and so does one sliding out under a Settings that came up at once
            if self.kinds.settings && self.settings.is_open() {
                self.panels.give_way(self.kit.modal_beat_ms() == 0);
            }
            self.panels.set_bell(self.home.bell_frame());
            self.panels.set_camera(self.home.camera_frame());
            self.panels.set_gear(self.home.gear_frame());
            self.panels.camera_focus(&self.kinds, self.kinds.home && self.home.camera_focused(), now);
            match self.panels.apply(&self.kinds, &mut self.kit, &mut self.engine, now, now_ns) {
                Ok(on) => live |= on,
                Err(e) => self.apply_failed("panels", e.to_string(), acts),
            }
            // the places of the switcher's cards, for the compositor to move its pictures to
            for e in self.panels.take_drawn() {
                self.carry(e, now, acts);
            }
            if self.kinds.home {
                match self.home_surface.apply(&mut self.kit, &mut self.engine, &mut self.home, now) {
                    Ok(on) => { live |= on; set_visible(&mut interactive, 4, on && self.home.active); },
                    Err(e) => self.apply_failed("home", format!("home: {e}"), acts),
                }
            }
            if self.kinds.cameras {
                match self.cameras.apply(&mut self.kit, &mut self.engine, now) {
                    Ok(on) => { live |= on; set_visible(&mut interactive, 8, on); },
                    Err(e) => self.apply_failed("cameras", format!("cameras: {e}"), acts),
                }
            }
            if self.kinds.settings {
                let now_ns = self.kit.ui.scene.now_ns();
                let over_video = self.sys.app_front();
                match self.settings.apply(&mut self.kit, &mut self.engine, over_video, now, now_ns) {
                    Ok(on) => { live |= on; set_visible(&mut interactive, 16, on); },
                    Err(e) => self.apply_failed("settings", format!("settings: {e}"), acts),
                }
            }
            if self.kinds.displaylab {
                let now_ns = self.kit.ui.scene.now_ns();
                match self.lab.apply(&mut self.kit, &mut self.engine, now_ns) {
                    Ok(on) => { live |= on; set_visible(&mut interactive, 32, on); },
                    // a lab that cannot be seen is not run: it ends, and the screen is put back
                    Err(e) => {
                        self.apply_failed("displaylab", format!("displaylab: {e}"), acts);
                        for fx in self.lab.close("it could not be drawn", now) {
                            self.carry(fx, now, acts);
                        }
                    }
                }
            }
            if self.kinds.guide {
                let now_ns = self.kit.ui.scene.now_ns();
                match self.guide.apply(&mut self.kit, &mut self.engine, now_ns) {
                    Ok(on) => { live |= on; set_visible(&mut interactive, 64, on); },
                    // a guide that cannot be seen is not run: it ends, and LG's own is the next press's
                    Err(e) => {
                        self.apply_failed("guide", format!("guide: {e}"), acts);
                        for fx in self.guide.close("it could not be drawn", now) {
                            self.carry(fx, now, acts);
                        }
                    }
                }
            }
            self.live = live;
            self.interactive_surfaces = interactive;
            let failed = std::mem::take(&mut self.apply_failed_now);
            self.apply_errs.retain(|(src, _)| failed.contains(src));
        }
        // a tick that drew nothing and asked nothing (a sky restating its paths) leaves the report as it was, which the sets of a Home at rest
        // built and compared 11 times a second for 0.18 ms of the host's thread each; a second at most goes by before it is looked at again
        let quiet = std::mem::take(&mut self.quiet_tick) && !redrew && acts.is_empty() && now.saturating_sub(self.reported_at) < REPORT_HEARTBEAT_MS;
        if self.started && !pointer_only && !quiet {
            #[cfg(test)]
            {
                self.reports_built += 1;
            }
            self.reported_at = now;
            let report = self.report();
            if self.reported.as_ref() != Some(&report) {
                self.reported = Some(report.clone());
                acts.push(Act::Channel(Outbound { method: "POST", path: "/v1/ui".into(), body: Some(report), tag: None, beside: false }));
            }
        }
    }

    /// What the cards and the pop-up ask that is not theirs to do: OK on the camera card opens the Cameras app's full view, OK on a card with no
    /// action of its own opens the notification drawer.
    fn layer_intents(&mut self, now: u64, acts: &mut Vec<Act>) {
        for i in self.sys.take_intents() {
            match i {
                crate::system::layer::Intent::OpenCamera(camera) if self.kinds.cameras => {
                    // the card's picture is the view's first: it opens on a picture, and the video replaces it when it plays
                    if let Some(pic) = self.sys.take_popup_picture() {
                        self.cameras.seed_full(&camera, pic);
                    }
                    for e in self.cameras.open_full(&camera, now) {
                        self.carry(e, now, acts);
                    }
                    self.cameras_asks(now, acts);
                    self.redraw = true;
                }
                crate::system::layer::Intent::OpenCamera(camera) => acts.push(Act::Log(format!("camera card: the full view of {camera} is not this process's"))),
                crate::system::layer::Intent::Snooze { camera, name } => {
                    let mut out = Vec::new();
                    self.panels.open_menu(crate::panels::menu::Spec::from_json(&crate::dnd::snooze_sheet(&name)), &self.kinds, now, &mut out);
                    for e in out {
                        self.carry(e, now, acts);
                    }
                    self.snoozing = Some((camera, name));
                }
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
        if self.kinds.displaylab {
            can.push("displaylab");
        }
        if self.kinds.guide {
            can.push("guide");
        }
        if self.kinds.toast {
            // not a panel: `ctl shell show toast [TEXT]` raises one by the feed's own path (see `channel`)
            can.push("toast");
        }
        can.extend(self.panels.can(&self.kinds));
        // Home in front wants the keys too: the compositor sends every key of the remote to the layer that wants them
        let wants = self.sys.wants(&self.kinds) || self.panels.wants(&self.kinds) || (self.kinds.home && self.home.active) || (self.kinds.cameras && self.cameras.wants()) || (self.kinds.settings && self.settings.wants()) || self.lab_wants() || self.guide_wants();
        let mut takes = self.sys.takes(&self.kinds);
        if self.lab_wants() {
            // the wheel's turns switch between the two looks: the compositor has no key for them until it is told they are the shell's
            takes.push("wheel");
        }
        if self.kinds.guide {
            // the Guide key while Live TV is in front (Live TV would launch LG's guide), and the rest of its keys while it is open: the compositor
            // has names for them but the app (Live TV) has them until told
            takes.extend(self.guide.takes());
        }
        if self.kinds.home {
            // the keys the compositor has no name for until it is told they are the shell's: the voice key while the helper can listen, the
            // on-screen remote's and Search's while Home is in front
            if self.sys.takes_voice() {
                takes.push("voice");
            }
            if self.home.active {
                takes.extend(["screenremote", "search"]);
            }
            // a smart-home card that has more than it shows scrolls with the turns of the wheel, and the weather view's map zooms with them; the
            // compositor sends them with their direction only to a client that names them
            if self.home.wheel_scrolls() || self.home.radar_takes_keys() {
                takes.extend(["wheelup", "wheeldown"]);
            }
            // the weather view's map zooms with the wheel and steps its loop with the channel keys
            if self.home.radar_takes_keys() {
                takes.extend(["chup", "chdown"]);
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
            s["format"] = self.formats.state();
        }
        for (k, val) in self.sys.state(&self.kinds).as_object().into_iter().flatten() {
            s[k] = val.clone();
        }
        s["panels"] = self.panels.state(&self.kinds);
        if self.kinds.home {
            s["home"] = self.home.state();
            s["home"]["silk"] = self.home_surface.silk_report().clone();
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
        if self.kinds.displaylab {
            for (k, val) in self.lab.state().as_object().into_iter().flatten() {
                s[k] = val.clone();
            }
        }
        if self.kinds.guide {
            for (k, val) in self.guide.state().as_object().into_iter().flatten() {
                s[k] = val.clone();
            }
        }
        s["live"] = json!(self.live);
        s["interactiveVisible"] = json!(self.sys.screen_on() && (self.interactive_surfaces != 0 || self.interactive_fades.iter().any(Option::is_some) || self.sys.interactive_visible() || self.panels.interactive_visible()));
        let room = self.kit.room();
        s["room"] = json!({"layers": room.layers, "groups": room.groups, "tracks": room.tracks});
        s["refusals"] = json!(self.kit.ui.scene.refusals());
        s
    }

    /// When the shell next wants a tick (the models' timers, a hop's pause).
    pub fn due_at(&self) -> Option<u64> {
        let mut due = None;
        if self.kinds.osd {
            due = [due, self.volume.due_at(), self.volume_surface.due_at()].into_iter().flatten().min();
        }
        if self.kinds.toast {
            due = [due, self.toasts.due_at(), self.formats.due_at()].into_iter().flatten().min();
        }
        if self.kinds.home {
            due = [due, self.home.due_at(), self.home.active.then(|| self.home_surface.due_at()).flatten()].into_iter().flatten().min();
        }
        let cameras = if self.kinds.cameras { self.cameras.due_at() } else { None };
        let settings = if self.kinds.settings { self.settings.due_at() } else { None };
        let lab = if self.kinds.displaylab { self.lab.due_at() } else { None };
        let guide = if self.kinds.guide { self.guide.due_at() } else { None };
        let refused = (self.kit.ui.scene.refusals() != self.refusals_seen).then_some(self.refusal_redraw_at);
        [due, self.sys.due_at(&self.kinds), self.panels.due_at(&self.kinds), cameras, settings, lab, guide, self.vwin.due_at(), self.release_at, self.icons_warm_at, self.glass_restyle_at, self.interactive_fades[0], self.interactive_fades[1], self.interactive_fades[2], refused].into_iter().flatten().min()
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    /// The volume pane's rectangle while it is up, and the toasts' cards (the host harness reads them).
    pub fn geometry(&self) -> Value {
        let mut g = json!({
            "osd": self.volume_surface.pane_rect().map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})),
            "toasts": self.toast_surface.rects().iter().map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})).collect::<Vec<_>>(),
            "format": self.badge_surface.rects().iter().map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})).collect::<Vec<_>>(),
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
        if self.kinds.displaylab {
            for (k, v) in self.lab.geometry(&self.engine).as_object().into_iter().flatten() {
                g[k] = v.clone();
            }
        }
        if self.kinds.guide {
            for (k, v) in self.guide.geometry(&self.engine).as_object().into_iter().flatten() {
                g[k] = v.clone();
            }
        }
        g
    }

    /// The layout and contrast lint of what the layer has on the screen (`--eval lint`): an object per finding.
    pub fn lint(&mut self) -> Value {
        let mut lint = crate::kit::lint::Lint::new();
        // the focused plate at full lift, whatever surface it is on: no clip above it may cut it
        let at = self.kit.ui.scene.now_ns();
        let lit: Vec<_> = self.engine.flow.lifted_ids(at).into_iter().filter_map(|id| self.engine.flow.lifted_plate(id, at).map(|(plate, clip)| (format!("focus {}", id.0), plate, clip))).collect();
        let report = |mut lint: crate::kit::lint::Lint| {
            for (name, plate, clip) in &lit {
                lint.lifted(name, *plate, *clip);
            }
            json!(lint.findings().iter().map(|f| json!({"kind": f.kind.name(), "subject": f.subject, "detail": f.detail})).collect::<Vec<_>>())
        };
        // the entry sheet's dim hides the screen it is over: only the sheet is read
        if self.panels.entry_up(&self.kinds) {
            self.panels.feed_lint(&self.kinds, &mut self.kit, &mut lint);
            return report(lint);
        }
        // the channel guide is a full page over everything but the system layer and the panels: while it is open (and no panel is over it) only it is read
        if self.kinds.guide && self.guide.is_open() && !self.panels.wants(&self.kinds) {
            self.guide.feed_lint(&mut self.kit, &mut lint);
            return report(lint);
        }
        // the display lab is an opaque page over everything but the system layer: while it is open only it is read
        if self.kinds.displaylab && self.lab.is_open() {
            self.lab.feed_lint(&mut self.kit, &mut lint);
            return report(lint);
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
        report(lint)
    }

    /// The volume model's own account (`--eval volume`).
    pub fn debug(&self) -> Value {
        self.volume.debug()
    }

    /// The theme or the text size changed: the surfaces are made again at the next draw, and the layout editor, when it is open, measures the
    /// surfaces it stands in for again (its stand-ins follow the person's text size, not the size they had when it was opened).
    pub fn restyle(&mut self) {
        self.restyle_surfaces(false);
    }

    fn restyle_surfaces(&mut self, glass_only: bool) {
        self.volume_surface.restyle();
        self.toast_surface.restyle();
        self.badge_surface.restyle();
        self.sys.restyle(&mut self.engine);
        self.panels.restyle(&mut self.engine);
        self.home_surface.restyle();
        self.cameras.restyle(&mut self.engine, self.kit.text_scale);
        if glass_only { self.settings.restyle_glass(&self.kit); } else { self.settings.restyle(); }
        self.lab.restyle(&mut self.engine);
        self.guide.restyle(&mut self.engine);
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
                // the full view is its own app: the app in front goes to the background as an app switch would, Home comes forward (LG's video path
                // connects the decoder's video for the app in front, and for none but Home here), and Back brings the app back
                crate::cameras::app::Intent::TakeForeground => {
                    self.cam_return = self.sys.foreground_app();
                    acts.push(Act::Log(format!("cameras: the full view takes the front from {}", self.cam_return.as_deref().unwrap_or("no app"))));
                    let payload = json!({"category": "home"});
                    self.carry(Effect::Luna { tag: crate::system::layer::LAUNCH_TAG, uri: "luna://com.webos.applicationManager/launchDefaultApp".into(), payload, subscribe: false }, now, acts);
                }
                // the full view is shown over an app that plays a video (its picture beside the app's, on the sub window): the app's video is paused for as
                // long as the view is up and plays again on Back, in the same place, because nothing took its window
                crate::cameras::app::Intent::PauseApp => {
                    if let Some(app) = self.sys.foreground_app() {
                        acts.push(Act::Log(format!("cameras: the full view opens beside {app}'s video, which is paused")));
                        self.carry(Effect::Channel { path: "/v1/appkey".into(), body: json!({"app": app, "key": "pause"}) }, now, acts);
                        self.cam_paused = Some(app);
                    }
                }
                crate::cameras::app::Intent::ResumeApp => {
                    if let Some(app) = self.cam_paused.take() {
                        acts.push(Act::Log(format!("cameras: Back plays {app}'s video again")));
                        self.carry(Effect::Channel { path: "/v1/appkey".into(), body: json!({"app": app, "key": "play"}) }, now, acts);
                    }
                }
                crate::cameras::app::Intent::ReturnToApp => {
                    if let Some(app) = self.cam_return.take() {
                        acts.push(Act::Log(format!("cameras: Back brings {app} back")));
                        // the compositor raises the app's own window; a launch would relaunch a web app that does not handle it (YouTube) at its start page
                        self.carry(Effect::Channel { path: "/v1/front".into(), body: json!({"app": app}) }, now, acts);
                    }
                }
            }
        }
        // a view that is gone by any other way (the Home key, the screen off) takes nothing back
        if !self.cameras.app.viewer_up() {
            self.cam_return = None;
            self.cam_paused = None;
        }
    }

    /// The Cameras app, for the screens that open it (the pop-up's OK, the quick panel's tile).
    pub fn cameras(&mut self) -> &mut Cameras {
        &mut self.cameras
    }

    /// What the screens want on the display's video windows now (the claims the compositor's `vwin` is asked for): the Cameras full view's live picture and
    /// the camera pop-up's. A screen with nothing live wants nothing, and the claim it had is released. A screen that wants a picture-in-picture adds its
    /// `Pip::request()` here.
    fn vwin_wants(&self) -> Vec<Value> {
        let live = if self.kinds.cameras { self.cameras.app.live_claim().map(|c| c.request()) } else { None };
        live.into_iter().chain(self.sys.popup_claim(&self.kinds)).collect()
    }

    fn open_wallpaper(&mut self, now: u64, acts: &mut Vec<Act>) {
        for e in self.settings.close(now) { self.carry(e, now, acts); }
        let mut out = Vec::new();
        self.home.edit_wallpaper(&mut out);
        for e in out { self.carry(e, now, acts); }
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
                    beside: false,
                })),
                ShellRequest::Layout { item } => {
                    if self.kinds.layout {
                        self.start_layout(item.as_deref());
                    } else {
                        acts.push(Act::Log("settings: the layout editor is not this process's".into()));
                    }
                }
                ShellRequest::DisplayLab => {
                    if self.kinds.displaylab {
                        for e in self.lab.open(now) {
                            self.carry(e, now, acts);
                        }
                        self.redraw = true;
                    } else {
                        acts.push(Act::Log("settings: the display lab was asked for and is not this process's".into()));
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
                ShellRequest::Wallpaper => {
                    if self.kinds.home {
                        if self.home.active {
                            self.open_wallpaper(now, acts);
                        } else if !self.wallpaper_waiting {
                            self.wallpaper_waiting = true;
                            self.carry(Effect::Luna { tag: crate::system::layer::LAUNCH_TAG, uri: "luna://com.webos.applicationManager/launchDefaultApp".into(), payload: json!({"category": "home"}), subscribe: false }, now, acts);
                        }
                    }
                }
                ShellRequest::Text { prompt, text, secret, purpose } => {
                    let mut out = Vec::new();
                    if self.panels.ask_text_purpose(crate::panels::layer::TextAsk { prompt: &prompt, text: &text, secret, purpose }, &self.kinds, now, &mut out) {
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
        if let Some(preview) = self.settings.take_theme_preview() {
            self.theme_preview = preview;
            self.home.set_theme_preview(self.theme_preview.clone());
            self.glass_restyle_at = None;
            self.apply_theme_look(now, acts, false);
        }
        if let Some(file) = self.settings.take_theme_file() && file != self.theme_base {
            self.theme_base = file;
            self.home.theme_text = self.theme_base.to_string();
            self.glass_restyle_at = None;
            self.apply_theme_look(now, acts, true);
        }
    }

    fn apply_theme_look(&mut self, now: u64, acts: &mut Vec<Act>, update_settings: bool) {
        if update_settings {
            let confirmed = lumatv_tokens::Tokens::from_json(&self.theme_base).json().clone();
            self.settings.set_theme(confirmed);
        }
        let mut file = self.theme_base.clone();
        if let Some(preview) = &self.theme_preview {
            crate::host::theme::merge_one_level(&mut file, preview);
        }
        let theme = lumatv_tokens::Tokens::from_json(&file).json().clone();
        if self.kit.tokens.json() == &theme && self.home.theme == theme {
            return;
        }
        let theme = self.reload_look(&file);
        self.home.set_theme(theme.clone());
        for e in self.panels.set_theme(&theme, now) {
            self.carry(e, now, acts);
        }
        for e in self.sys.set_theme(&theme, now) {
            self.carry(e, now, acts);
        }
    }

    /// The owner placed the pop-ups somewhere else (`layout.overlays`): every surface that has a place reads it from now on.
    fn set_places(&mut self, theme: &Value) {
        let user = lumatv_placement::Registry::shipped().user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        self.volume_surface.set_user(user.clone());
        self.toast_surface.set_user(user.clone());
        self.badge_surface.set_user(user.clone());
        self.sys.set_user(user);
    }

    /// The owner's theme file changed (Settings wrote it): the look in force is the shipped theme under it. Returns the merged theme.
    pub fn reload_theme(&mut self, file: &Value) -> Value {
        self.glass_restyle_at = None;
        let theme = self.reload_look(file);
        self.settings.set_theme(theme.clone());
        theme
    }

    fn reload_look(&mut self, file: &Value) -> Value {
        let tokens = lumatv_tokens::Tokens::from_json(file);
        let mode = match tokens.material {
            lumatv_tokens::Material::Dark => ui::lucent::palette::Mode::Dark,
            lumatv_tokens::Material::Light => ui::lucent::palette::Mode::Light,
            lumatv_tokens::Material::Auto { .. } => self.kit.mode,
        };
        let scale = file.pointer("/type/scale").and_then(Value::as_f64).map_or(self.theme_default_scale, |s| (s as f32).clamp(1.0, 2.0));
        let theme = tokens.json().clone();
        self.kit.retheme(tokens, mode, scale);
        self.cameras.app.cfg = CamerasConfig::from_theme(&self.kit, &theme);
        self.set_places(&theme);
        self.restyle_surfaces(self.glass_restyle_at.is_some());
        theme
    }
}

#[cfg(test)]
mod displaylab_tests;

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
    fn settings_promotion_during_close_releases_the_surface_and_vote() {
        use compositor_protocol::{Request, motion::Settle};
        let (mut h, mut shell) = home_shell("home,settings");
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 2_000);
        shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 60}) }, 2_010);
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "settings"}) }, 2_020);
        h.commit();
        let mut acts = Vec::new();
        for effect in shell.settings.close(2_030) { shell.carry(effect, 2_030, &mut acts); }
        shell.after(2_030, &mut acts);
        assert_eq!(shell.state()["interactiveVisible"], true, "the 60 Hz close still fades");
        let closing = h.commit();
        let fade = closing.iter().find_map(|r| match r {
            Request::MotionSettle { id, target: Some(0.0), settle: Settle::Timed(token), .. } if token.duration_ms > 0 => Some(*id),
            _ => None,
        }).expect("Settings started its 60 Hz closing fade");
        shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 120}) }, 2_040);
        let finished = h.commit();
        // Dropping the surface removes its track, so no close animation remains in flight.
        assert!(finished.iter().any(|r| matches!(r, Request::MotionDelete { id } if *id == fade)), "the fade track is released");
        assert_eq!(shell.state()["interactiveVisible"], false);
    }

    #[test]
    fn settings_reopen_during_fade_keeps_interactive_vote() {
        for promote in [false, true] {
            let (_h, mut shell) = home_shell("home,settings");
            shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 2_000);
            shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 60}) }, 2_010);
            assert_eq!(shell.state()["interactiveVisible"], false);
            shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "settings"}) }, 2_020);
            let mut acts = Vec::new();
            for effect in shell.settings.close(2_030) { shell.carry(effect, 2_030, &mut acts); }
            shell.after(2_030, &mut acts);
            shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "settings"}) }, 2_040);
            if promote {
                shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 120}) }, 2_050);
            }
            shell.tick(3_000);
            assert_eq!(shell.state()["interactiveVisible"], true, "reopened Settings keeps its vote past the old close deadline");
        }
    }

    #[test]
    fn interactive_visibility_follows_native_menu_until_it_finishes_leaving() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("menu"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        assert_eq!(shell.state()["interactiveVisible"], false);
        shell.channel(&Event::Topic { topic: "menu.open".into(), data: json!({"items": [{"id": "a", "label": "A"}]}) }, 1_000);
        assert_eq!(shell.state()["interactiveVisible"], true);
        key(&mut shell, "back", 1_100);
        assert_eq!(shell.state()["interactiveVisible"], true);
        shell.tick(2_000);
        assert_eq!(shell.state()["interactiveVisible"], false);
    }

    #[test]
    fn home_votes_only_in_foreground_with_screen_on() {
        let (_h, mut shell) = home_shell("home");
        assert_eq!(shell.state()["interactiveVisible"], true);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 2_000);
        assert_eq!(shell.state()["interactiveVisible"], false);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 3_000);
        assert_eq!(shell.state()["interactiveVisible"], true);
        shell.channel(&Event::Topic { topic: "screen".into(), data: json!({"on": false}) }, 4_000);
        assert_eq!(shell.state()["interactiveVisible"], false);
    }
    #[test]
    fn interactive_visibility_tracks_drawn_keyboard_screen_and_reconnect() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("keyboard"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        assert_eq!(shell.state()["interactiveVisible"], false);
        shell.channel(&Event::Topic { topic: "keyboard".into(), data: json!({"up": true}) }, 1_000);
        assert_eq!(shell.state()["interactiveVisible"], false, "external keyboard latch is not a drawn keyboard");
        shell.channel(&Event::Topic { topic: "ime".into(), data: json!({"active": true, "show": true, "model": 1}) }, 1_100);
        assert_eq!(shell.state()["interactiveVisible"], true);
        let acts = shell.channel(&Event::State(Vec::new()), 1_200);
        assert!(acts.iter().any(|a| matches!(a, Act::Channel(o) if o.path == "/v1/ui" && o.body.as_ref().is_some_and(|b| b["state"]["interactiveVisible"] == true))));
        let acts = shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 60}) }, 1_250);
        assert!(acts.iter().any(|a| matches!(a, Act::Channel(o) if o.path == "/v1/ui" && o.body.as_ref().is_some_and(|b| b["state"]["interactiveVisible"] == true))));
        shell.channel(&Event::Topic { topic: "screen".into(), data: json!({"on": false}) }, 1_300);
        assert_eq!(shell.state()["interactiveVisible"], false);
    }

    #[test]
    fn interactive_visibility_distinguishes_volume_pill_mute_mark_and_toast() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("osd,toast"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        let audio = |cause| Event::Topic { topic: "audio".into(), data: json!({"volumeStatus": {"volume": 31, "muteStatus": true, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": cause}}) };
        shell.channel(&audio(""), 1_000);
        assert_eq!(shell.state()["mark"], true);
        assert_eq!(shell.state()["interactiveVisible"], false);
        shell.channel(&audio("volumeUp"), 1_100);
        assert_eq!(shell.state()["interactiveVisible"], true);
        shell.tick(10_000);
        assert_eq!(shell.state()["mark"], true);
        shell.tick(10_500);
        assert_eq!(shell.state()["interactiveVisible"], false);
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "toast", "n": 1}) }, 11_000);
        assert_eq!(shell.state()["interactiveVisible"], true);
        shell.tick(20_000);
        shell.tick(20_500);
        assert_eq!(shell.state()["interactiveVisible"], false);
    }

    #[test]
    fn do_not_disturb_holds_an_outside_senders_toast_for_the_helper_and_never_the_volume_bar() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("osd,toast"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        shell.helper(&Event::Topic { topic: crate::dnd::TOPIC.into(), data: json!({"on": true}) }, 1_000);
        let ha = json!({"tag": FEED_TAG, "payload": {"sourceId": "com.webos.service.secondscreen.gateway", "message": "Washer<br>Done"}});
        let acts = shell.channel(&Event::Topic { topic: "luna".into(), data: ha }, 1_100);
        assert_eq!((helper_calls(&acts, "/v1/dnd/missed"), shell.state()["toasts"].clone()), (1, json!(0)));
        let audio = json!({"volumeStatus": {"volume": 31, "muteStatus": false, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": "volumeUp"}});
        shell.channel(&Event::Topic { topic: "audio".into(), data: audio }, 1_200);
        assert_eq!(shell.state()["osd"], true);
    }

    #[test]
    fn volume_consumes_subtitle_bounds_from_the_channel() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("osd"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app":"player", "surface":1, "home":false}) }, 1_000);
        shell.channel(&Event::Topic { topic: "subtitleBounds".into(), data: json!({"app":"player", "surface":1, "rect":null}) }, 1_001);
        let audio = |cause| Event::Topic { topic: "audio".into(), data: json!({"volumeStatus":{"volume":31,"muteStatus":false,"soundOutput":"tv_speaker","maxVolume":100,"cause":cause}}) };
        shell.channel(&audio(""), 1_002);
        shell.channel(&audio("volumeUp"), 1_003);
        let base = shell.volume_surface.pane_rect().unwrap();
        shell.channel(&Event::Topic { topic: "subtitleBounds".into(), data: json!({"app":"player", "surface":1, "rect":{"x":0,"y":900,"w":1920,"h":150}}) }, 1_004);
        let raised = shell.volume_surface.pane_rect().unwrap();
        assert!(raised.y < base.y);
        assert!(raised.y + raised.h + shell.kit.space(2) <= 900.0);
        shell.channel(&Event::Topic { topic: "subtitleBounds".into(), data: json!({"app":"player", "surface":1, "rect":null}) }, 1_005);
        assert_eq!(shell.volume_surface.pane_rect().unwrap().y, raised.y);
    }


    #[test]
    fn the_guide_key_is_listed_while_live_tv_is_in_front_and_opens_the_guide_without_live_tv_seeing_it() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("guide"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        let front = |app: &str| Event::Topic { topic: "foreground".into(), data: json!({"app": app, "home": false, "covered": true}) };
        let guide = |down| KeyEvent::Key { name: "guide".into(), down, repeat: false, seat: 0 };
        shell.channel(&front("org.test.player"), 1_000);
        assert_eq!(shell.report()["takes"], json!([]), "another app in front: the key is the app's");
        shell.key(&guide(true), 1_100);
        assert_eq!(shell.state()["guide"]["open"], false, "a key the compositor should not have sent opens nothing");
        shell.channel(&front("com.webos.app.livetv"), 2_000);
        assert_eq!(shell.report()["takes"], json!(["guide"]));
        shell.key(&guide(true), 2_100);
        shell.key(&guide(false), 2_160);
        assert_eq!(shell.state()["guide"]["open"], true);
        assert!(shell.report()["takes"].as_array().unwrap().iter().any(|k| k == "guide"), "still claimed while it is open");
        shell.channel(&front("org.test.player"), 3_000);
        assert_eq!(shell.state()["guide"]["open"], false);
        assert_eq!(shell.report()["takes"], json!([]));
    }

    #[test]
    fn the_compositors_show_for_a_launch_of_lgs_guide_opens_the_guide_over_whatever_is_in_front_and_a_second_asks_nothing() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("guide"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).unwrap();
        shell.start();
        let front = |app: &str| Event::Topic { topic: "foreground".into(), data: json!({"app": app, "home": false, "covered": true}) };
        let show = |n| Event::Topic { topic: "show".into(), data: json!({"kind": "guide", "warm": false, "n": n}) };
        let asks = |acts: &[Act]| acts.iter().filter(|a| matches!(a, Act::Channel(o) if o.path == "/v1/luna")).count();
        shell.channel(&front("org.test.player"), 1_000);
        let first = shell.channel(&show(1), 1_100);
        assert_eq!(shell.state()["guide"]["open"], true, "LG's guide was launched over an app that is not Live TV: this one opens over it");
        assert!(asks(&first) > 0, "and asks LG's guide service for the channels");
        let second = shell.channel(&show(2), 1_200);
        assert!(shell.state()["guide"]["open"] == true && asks(&second) == 0, "a second launch while it is open asks nothing");
        shell.channel(&front("org.test.player"), 1_300);
        assert_eq!(shell.state()["guide"]["open"], true, "the app it came over is still in front: it stays");
        shell.channel(&front("com.webos.app.home"), 2_000);
        assert_eq!(shell.state()["guide"]["open"], false, "another app coming to the front ends it, as ever");
    }

    #[test]
    fn loaded_legacy_custom_colours_do_not_claim_the_shipped_palette() {
        let (_h, mut shell) = shell();
        let mut acts = Vec::new();
        shell.feed(&Input::Fetched { tag: crate::home::model::THEME_TAG, status: 200,
            data: json!({"dark": {"accent": "#010203"}, "palette": {"light": "other"}}) }, 1, &mut acts);
        for theme in [shell.kit.tokens.json(), &shell.home.theme] {
            assert_eq!(theme["palette"]["dark"], "");
            assert_eq!(theme["palette"]["light"], "other");
            assert_eq!(theme["dark"]["accent"], "#010203");
        }
    }
    #[test]
    fn rapid_glass_writes_keep_sibling_settings_and_restyle_once() {
        let (_h, mut shell) = home_shell("home,settings");
        let dir = std::env::temp_dir().join(format!("lumatv-glass-writes-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        let original = json!({"wallpaperMotion": {"enabled": false}, "motion": {"transitions": true}, "glassStrength": {"clear": 0.8, "liquid": 0.4}});
        std::fs::write(dir.join("theme/theme.json"), original.to_string()).unwrap();
        shell.dirs(&testkit::module_dir(), &dir);
        let mut acts = Vec::new();
        shell.theme_base = original;
        shell.apply_theme_look(0, &mut acts, true);
        shell.settings.open(Some("look"), 1);
        shell.settings.click(crate::settings::model::Target::Row { id: "glassstrength".into(), seg: None, value: None }, 10);
        let generation = shell.kit.generation();
        for (now, value, repeat) in [(100, 0.79, false), (150, 0.78, true), (200, 0.77, true)] {
            shell.key(&KeyEvent::Key { name: "left".into(), down: true, repeat, seat: 0 }, now);
            let page = shell.settings.model.view().page.unwrap();
            assert_eq!(page.rows.iter().find(|r| r.id == "glassstrength").unwrap().num, (value * 100.0_f64).round());
            assert!(page.rows.iter().any(|r| r.id == "wallpaper"));
            assert!(page.rows.iter().find(|r| r.id == "transitions").unwrap().on);
            assert_eq!(shell.kit.generation(), generation);
            assert_eq!(shell.kit.tokens.json()["glassStrength"]["clear"], value);
        }
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.join("theme/theme.json")).unwrap()).unwrap();
        assert_eq!(saved["glassStrength"], json!({"clear": 0.77, "liquid": 0.4}));
        assert_eq!(saved["wallpaperMotion"]["enabled"], false);
        assert_eq!(saved["motion"]["transitions"], true);
        assert_eq!(shell.glass_restyle_at, Some(550));
        shell.tick(549);
        assert_eq!(shell.kit.generation(), generation);
        shell.tick(550);
        assert_eq!(shell.kit.generation(), generation + 1);
        assert_eq!(shell.kit.tokens.json()["glassStrength"]["clear"], 0.77);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn glass_write_keeps_and_applies_an_external_sibling_change() {
        let (_h, mut shell) = home_shell("home,settings");
        let dir = std::env::temp_dir().join(format!("lumatv-glass-external-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        let original = json!({"wallpaperMotion": {"enabled": false}, "motion": {"transitions": true}});
        std::fs::write(dir.join("theme/theme.json"), original.to_string()).unwrap();
        shell.dirs(&testkit::module_dir(), &dir);
        let mut acts = Vec::new();
        shell.theme_base = original;
        shell.apply_theme_look(0, &mut acts, true);
        shell.write_theme(9999, &json!({"glassStrength": {"clear": 0.8}}), 100, &mut acts);
        crate::host::theme::persist(&dir, &json!({"wallpaperMotion": {"enabled": true}})).unwrap();
        shell.write_theme(9999, &json!({"glassStrength": {"clear": 0.7}}), 150, &mut acts);
        assert_eq!(shell.home.theme["wallpaperMotion"]["enabled"], true);
        assert_eq!(shell.home.theme["motion"]["transitions"], true);
        assert_eq!(shell.home.theme["glassStrength"]["clear"], 0.7);
        assert!(shell.glass_restyle_at.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }


    #[test]
    fn preview_tracks_external_theme_changes_and_cancel_restores_them() {
        let (_h, mut shell) = shell();
        let mut acts = Vec::new();
        shell.theme_base = json!({"type": {"scale": 1.0}, "clock": {"style": "digital"}});
        shell.theme_preview = Some(json!({"type": {"scale": 1.5}}));
        shell.home.set_theme_preview(shell.theme_preview.clone());
        shell.apply_theme_look(0, &mut acts, false);
        assert_eq!(shell.kit.text_scale, 1.5);
        shell.feed(&Input::Fetched { tag: crate::home::model::THEME_TAG, status: 200, data: json!({"type": {"scale": 1.0}, "clock": {"style": "analog"}}) }, 1, &mut acts);
        assert_eq!(shell.kit.text_scale, 1.5);
        assert_eq!(shell.home.theme["clock"]["style"], "analog");
        shell.theme_preview = None;
        shell.home.set_theme_preview(None);
        shell.apply_theme_look(2, &mut acts, false);
        assert_eq!(shell.kit.text_scale, 1.0);
        assert_eq!(shell.home.theme["clock"]["style"], "analog");
    }

    #[test]
    fn a_failure_that_repeats_every_frame_is_logged_once_and_again_after_it_clears() {
        let (_h, mut shell) = shell();
        let mut acts = Vec::new();
        let pass = |shell: &mut Shell, text: Option<&str>, acts: &mut Vec<Act>| {
            if let Some(t) = text {
                shell.apply_failed("settings", t.into(), acts);
            }
            let failed = std::mem::take(&mut shell.apply_failed_now);
            shell.apply_errs.retain(|(src, _)| failed.contains(src));
        };
        for _ in 0..1000 {
            pass(&mut shell, Some("settings: at most 96 groups"), &mut acts);
        }
        assert_eq!(acts.len(), 1);
        pass(&mut shell, Some("settings: another"), &mut acts);
        assert_eq!(acts.len(), 2);
        pass(&mut shell, None, &mut acts);
        pass(&mut shell, Some("settings: another"), &mut acts);
        assert_eq!(acts.len(), 3, "it came back after a good frame");
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

    #[test]
    fn wallpaper_from_look_previews_and_saves_then_returns_to_look() {
        let (_h, mut shell) = home_shell("home,settings");
        let dir = std::env::temp_dir().join(format!("lumatv-wallpaper-route-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        std::fs::write(dir.join("theme/theme.json"), r#"{"wallpaperMotion":{"enabled":true,"speed":1.2},"custom":{"kept":true}}"#).unwrap();
        shell.dirs(&testkit::module_dir(), &dir);
        shell.open_settings(Some("look"), 2_000);
        shell.settings.click(crate::settings::model::Target::Row { id: "wallpaper".into(), seg: None, value: None }, 2_010);
        shell.after(2_010, &mut Vec::new());
        assert!(!shell.settings.is_open());
        assert_eq!(shell.home.state()["layer"], "popover");
        assert_eq!(shell.home.state()["popover"]["row"], "enabled");
        key(&mut shell, "ok", 2_100);
        assert_eq!(shell.home.theme["wallpaperMotion"]["enabled"], false);
        key(&mut shell, "back", 2_200);
        assert!(shell.settings.is_open());
        assert_eq!(shell.settings.model.view().page.unwrap().id, "look");
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.join("theme/theme.json")).unwrap()).unwrap();
        assert_eq!(saved["wallpaperMotion"]["enabled"], false);
        assert_eq!(saved["wallpaperMotion"]["speed"], 1.2);
        assert_eq!(saved["custom"]["kept"], true);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wallpaper_waits_for_home_before_opening_over_an_app() {
        let (_h, mut shell) = home_shell("home,settings");
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 2_000);
        shell.open_settings(Some("look"), 2_100);
        shell.settings.click(crate::settings::model::Target::Row { id: "wallpaper".into(), seg: None, value: None }, 2_110);
        let mut acts = Vec::new();
        shell.after(2_110, &mut acts);
        assert!(shell.settings.is_open());
        assert_eq!(shell.home.state()["layer"], "");
        assert!(luna_calls(&acts, "launchDefaultApp").iter().any(|call| call["payload"]["category"] == "home"));
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 2_200);
        assert!(!shell.settings.is_open());
        assert_eq!(shell.home.state()["layer"], "popover");
        key(&mut shell, "ok", 2_300);
        assert_eq!(shell.home.theme["wallpaperMotion"]["enabled"], false);
        key(&mut shell, "back", 2_400);
        assert!(shell.settings.is_open());
        assert_eq!(shell.settings.model.view().page.unwrap().id, "look");
    }

    #[test]
    fn wallpaper_forced_close_saves_without_reopening_settings() {
        let (_h, mut shell) = home_shell("home,settings");
        let dir = std::env::temp_dir().join(format!("lumatv-wallpaper-exit-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        std::fs::write(dir.join("theme/theme.json"), r#"{"wallpaperMotion":{"enabled":true}}"#).unwrap();
        shell.dirs(&testkit::module_dir(), &dir);
        shell.open_settings(Some("look"), 2_000);
        shell.settings.click(crate::settings::model::Target::Row { id: "wallpaper".into(), seg: None, value: None }, 2_010);
        shell.after(2_010, &mut Vec::new());
        key(&mut shell, "ok", 2_100);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 2_200);
        assert!(!shell.settings.is_open());
        assert_eq!(shell.home.state()["layer"], "");
        assert!(shell.home.take_requests().is_empty());
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.join("theme/theme.json")).unwrap()).unwrap();
        assert_eq!(saved["wallpaperMotion"]["enabled"], false);
        std::fs::remove_dir_all(dir).unwrap();
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
    fn the_saver_votes_is_up_once_tvpower_asks_and_any_key_ends_it() {
        let (_h, mut shell) = home_shell("home,saver");
        shell.tick(1_100);
        // tvpower takes both lists, then asks on the first: the answer is no, and the saver is up
        for tag in [0x5359_0010u64, 0x5359_0011] {
            shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": tag, "payload": {"returnValue": true, "subscribed": true}}) }, 1_200);
        }
        assert_eq!(shell.state()["saverState"]["vote"]["registered"], true);
        shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": 0x5359_0010u64, "payload": {"returnValue": true, "timestamp": "1"}}) }, 180_000);
        shell.tick(180_050);
        assert_eq!(shell.state()["saver"], true, "the question was answered no: {}", shell.state()["saverState"]);
        assert_eq!(shell.state()["saverState"]["owned"], true);
        assert_eq!(shell.state()["interactiveVisible"], true, "Home remains a pacing demand while retained under the saver");
        key(&mut shell, "left", 181_000);
        assert_eq!(shell.state()["saver"], false, "a key ends it");
    }

    #[test]
    fn native_saver_wake_press_repeat_and_release_do_not_close_the_panel() {
        let (_h, mut shell) = home_shell("home,saver,quick");
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 1_900);
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "quick", "n": 1}) }, 2_000);
        shell.tick(2_300);
        assert_eq!(shell.state()["panels"]["quick"], "open");
        shell.helper(&Event::Topic { topic: "screen".into(), data: json!({"on": true, "state": "Screen Saver"}) }, 2_500);
        assert_eq!(shell.state()["saver"], true);
        assert_eq!(shell.state()["panels"]["quick"], "open", "panel remains open under the native saver");
        let event = |down, repeat| KeyEvent::Key { name: "back".into(), down, repeat, seat: 0 };
        let acts = shell.key(&event(true, false), 2_600);
        assert_eq!(luna_calls(&acts, "/power/turnOnScreen").len(), 1);
        assert_eq!(shell.state()["saver"], false);
        shell.key(&event(true, true), 2_650);
        shell.key(&event(false, false), 2_700);
        assert_eq!(shell.state()["panels"]["quick"], "open", "wake key and release must not close it");
        key(&mut shell, "back", 2_800);
        shell.tick(3_100);
        assert_ne!(shell.state()["panels"]["quick"], "open", "the next press reaches the panel");
    }

    #[test]
    fn native_saver_pointer_wake_does_not_click_the_panel_under_it() {
        let (_h, mut shell) = home_shell("home,saver,quick");
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false, "covered": true}) }, 1_900);
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "quick", "n": 1}) }, 2_000);
        shell.tick(2_300);
        shell.helper(&Event::Topic { topic: "screen".into(), data: json!({"on": true, "state": "Screen Saver"}) }, 2_500);
        assert_eq!(shell.state()["saver"], true);
        assert_eq!(shell.state()["panels"]["quick"], "open");
        let pointer = |buttons| KeyEvent::Pointer { x: 300.0, y: 300.0, buttons, visible: true };
        shell.key(&pointer(0), 2_550);
        assert_eq!(shell.state()["saver"], true, "the takeover sample is not a wake");
        let acts = shell.key(&pointer(1), 2_600);
        assert_eq!(luna_calls(&acts, "/power/turnOnScreen").len(), 1);
        assert_eq!(shell.state()["saver"], false);
        shell.key(&pointer(0), 2_650);
        assert_eq!(shell.state()["panels"]["quick"], "open", "the waking click did not reach the panel");
        shell.key(&pointer(1), 2_700);
        shell.key(&pointer(0), 2_750);
        shell.tick(3_000);
        assert_ne!(shell.state()["panels"]["quick"], "open", "a later click reaches the panel");
    }

    #[test]
    fn voice_wakes_native_saver_without_starting_or_stopping_voice_search() {
        let (_h, mut shell) = home_shell("home,saver");
        shell.helper(&Event::Up(true), 1_100);
        shell.helper(&Event::Topic { topic: "voice".into(), data: json!({"ready": true, "state": "idle"}) }, 1_200);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "org.test.player", "home": false}) }, 1_300);
        shell.helper(&Event::Topic { topic: "screen".into(), data: json!({"on": true, "state": "Screen Saver"}) }, 1_400);
        assert_eq!(shell.state()["saver"], true);
        let acts = key(&mut shell, "voice", 1_500);
        assert_eq!(luna_calls(&acts, "/power/turnOnScreen").len(), 1);
        assert_eq!(helper_calls(&acts, "/v1/voice/listen"), 0);
        assert_eq!(helper_calls(&acts, "/v1/voice/stop"), 0);
        assert_eq!(shell.state()["extras"]["voice"], "");
        let acts = shell.key(&KeyEvent::Key { name: "voice".into(), down: true, repeat: false, seat: 0 }, 1_600);
        assert_eq!(helper_calls(&acts, "/v1/voice/listen"), 1, "ordinary voice still starts listening");
    }

    #[test]
    fn an_app_that_opens_from_home_shows_the_launch_screen_after_the_compositors_wait_and_tells_it_where_the_icon_is_and_it_goes_when_the_app_has_the_front() {
        let (_h, mut shell) = home_shell("home");
        shell.channel(&Event::Topic { topic: "transitions".into(), data: json!({"on": true, "reduced": false}) }, 1_020);
        for (i, k) in ["down", "left", "left", "down"].into_iter().enumerate() {
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

    fn pointer(shell: &mut Shell, (x, y): (f32, f32), buttons: u32, visible: bool, at: u64) -> Vec<Act> {
        shell.key(&KeyEvent::Pointer { x, y, buttons, visible }, at)
    }

    #[test]
    fn nothing_opens_by_a_button_nobody_saw_go_down_or_a_pointer_that_went_to_sleep() {
        let (_h, mut shell) = home_shell("home");
        let r = shell.home.dock_rect().expect("the dock");
        let (sx, sy) = shell.home.dock.slot(1);
        let over = (r.x as f32 + sx as f32 + 60.0, r.y as f32 + sy as f32 + 60.0);
        let mut acts = Vec::new();
        // the remote wakes with the button reading down, then lies still and sleeps with the button up, shown and hidden samples mixed
        for (buttons, visible, at) in [(1, true, 2_000), (1, true, 2_020), (0, true, 2_040), (0, false, 2_060), (1, false, 2_080), (0, false, 2_100), (0, true, 2_120)] {
            acts.extend(pointer(&mut shell, over, buttons, visible, at));
        }
        assert!(luna_calls(&acts, "/launch").is_empty(), "{acts:?}");
        // the real thing still works: seen up, down, up over the tile
        for (buttons, at) in [(0, 2_200), (1, 2_220), (0, 2_260)] {
            acts.extend(pointer(&mut shell, over, buttons, true, at));
        }
        assert_eq!(luna_calls(&acts, "/launch").len(), 1, "{acts:?}");
    }

    #[test]
    fn without_the_compositors_transitions_the_launch_screen_is_there_at_once_as_it_always_was() {
        let (_h, mut shell) = home_shell("home");
        for (i, k) in ["down", "left", "left", "down"].into_iter().enumerate() {
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
            key(&mut shell, "left", 2_100 + i * 100);
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
        // Up from the keyboard reaches the answers; left and right choose one.
        for i in 0..6 {
            if shell.home.search.answer_index().is_some() {
                break;
            }
            key(&mut shell, "up", 2_200 + i * 100);
        }
        for i in 0..4 {
            let at = shell.home.search.answer_index().expect("an answer is focused");
            if at == page {
                break;
            }
            key(&mut shell, if at > page { "left" } else { "right" }, 3_000 + i * 100);
        }
        assert_eq!(shell.home.search.answer_index(), Some(page));
        key(&mut shell, "ok", 4_000);
        assert_eq!(shell.state()["settings"]["open"], true);
        assert_eq!(shell.state()["settings"]["page"], "network");
        assert_eq!(shell.state()["home"]["layer"], "", "Home's sheet is closed behind it");
    }

    #[test]
    fn ok_on_the_bell_opens_the_drawer_and_back_closes_it_and_without_the_drawer_kind_nothing_opens() {
        let (_h, mut shell) = home_shell("home,drawer");
        key(&mut shell, "right", 2_000);
        key(&mut shell, "right", 2_100);
        key(&mut shell, "ok", 2_200);
        assert!(matches!(shell.state()["panels"]["drawer"].as_str(), Some("opening" | "open")), "{}", shell.state()["panels"]);
        key(&mut shell, "back", 2_600);
        assert!(matches!(shell.state()["panels"]["drawer"].as_str(), Some("leaving" | "parked")), "{}", shell.state()["panels"]);
        assert_eq!(shell.state()["home"]["focusLayer"], "statusbar", "Home is where it was left");
        let (_h2, mut bare) = home_shell("home");
        key(&mut bare, "right", 2_000);
        key(&mut bare, "right", 2_100);
        key(&mut bare, "ok", 2_200);
        assert!(!matches!(bare.state()["panels"]["drawer"].as_str(), Some("opening" | "open")));
    }

    /// What the helper reports of the people of this TV: Who's watching has two tiles.
    fn profiles_topic() -> Value {
        let person = |id: &str, name: &str, active: bool| json!({"id": id, "name": name, "initial": &name[..1], "color": "#A79BF5", "avatar": {"kind": "initial"}, "avatarFile": null,
                                                                  "owner": active, "active": active, "locked": false, "firstRun": false});
        json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "pickerOnWake": false, "startWith": "last", "max": 6, "switchedAt": 1,
               "profiles": [person("nitin", "Nitin", true), person("mia", "Mia", false)]})
    }

    #[test]
    fn every_popup_of_the_bar_grows_out_of_its_buttons_laid_out_frame_wherever_the_pill_sits_and_its_body_stays_on_the_screen() {
        let numbers = |v: &Value, what: &str| -> Vec<f64> { v.as_array().unwrap_or_else(|| panic!("{what}: a rectangle, not {v}")).iter().map(|n| n.as_f64().unwrap()).collect() };
        let centre = |r: &[f64]| (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        for (anchor, dx, full) in [("top-left", 0.0, false), ("top", 0.0, false), ("top-right", 0.0, false), ("top", 0.17, false), ("top-left", 0.0, true), ("top", 0.0, true), ("top-right", 0.0, true), ("top", 0.17, true)] {
            let placed = json!({"layout": {"overlays": {"statusbar": {"anchor": anchor, "dx": dx, "dy": 0, "full": full}}}});
            let anchor = if full { format!("{anchor} full width") } else { anchor.to_owned() };
            let (_h, mut shell) = home_shell_with("home,drawer,quick,cameras", &placed);
            shell.helper(&Event::Up(true), 1_050);
            shell.helper(&Event::Topic { topic: "profiles".into(), data: profiles_topic() }, 1_100);
            let cameras: Vec<Value> = (0..8).map(|i| json!({"id": format!("c{i}"), "name": format!("Camera {i}"), "online": true})).collect();
            shell.helper(&Event::Topic { topic: "cameras".into(), data: json!({"state": "ready", "cameras": cameras}) }, 1_120);
            let (mut at, mut t) = (0usize, 2_000u64);
            // the four popups in the order of the bar: each from its own button by OK, seen drawn, and sent back with Back
            for (kind, surface) in [("cameras", "cameraList"), ("notifications", "drawer"), ("settings", "quick"), ("profile", "home")] {
                let i = shell.home.status_index(kind).expect(kind);
                for _ in at..i {
                    key(&mut shell, "right", t);
                    t += 100;
                }
                at = i;
                // the button where the bar lays it out now
                let b = shell.home.status_frames()[i];
                key(&mut shell, "ok", t);
                t += 400;
                let g = shell.geometry();
                let origin = numbers(if kind == "profile" { &g["home"]["pickerOrigin"] } else { &g[surface]["origin"] }, &format!("{kind} at {anchor} {dx}"));
                assert_eq!(centre(&origin), centre(&[f64::from(b.0), f64::from(b.1), f64::from(b.2), f64::from(b.3)]), "{kind} at {anchor} {dx}: grows out of its button {b:?}, not {origin:?}");
                // the body is under its button: its middle under the button's (against the screen's margin where the button is near one), below the
                // pill, and whole on the screen
                if let Some(sheet) = g[surface].get("sheet").filter(|s| s.is_object()) {
                    let n = |k: &str| sheet[k].as_f64().unwrap();
                    let (x, y, w, h) = (n("x"), n("y"), n("w"), n("h"));
                    let want = centre(&[f64::from(b.0), f64::from(b.1), f64::from(b.2), f64::from(b.3)]).0;
                    if want - w / 2.0 < 40.0 {
                        assert_eq!(x, 40.0, "{kind} at {anchor} {dx}: against the left margin: {sheet}");
                    } else if want + w / 2.0 > 1880.0 {
                        assert_eq!(x + w, 1880.0, "{kind} at {anchor} {dx}: against the right margin: {sheet}");
                    } else {
                        assert!((x + w / 2.0 - want).abs() <= 0.5, "{kind} at {anchor} {dx}: the body's middle {} is under the button's {want}: {sheet}", x + w / 2.0);
                    }
                    let pill = shell.home.status_pill();
                    assert!(y >= f64::from(pill.y + pill.h), "{kind} at {anchor} {dx}: the body starts under the pill: {sheet}");
                    assert!(x >= 0.0 && x + w <= 1920.0 && y + h <= 1080.0 - 40.0 + 0.5, "{kind} at {anchor} {dx}: the body is whole on the screen: {sheet}");
                }
                // a list taller than the sheet still scrolls with the pill where it is
                if kind == "cameras" {
                    for _ in 0..7 {
                        key(&mut shell, "down", t);
                        t += 100;
                    }
                    let list = shell.geometry()["cameraList"].clone();
                    assert!(list["offset"].as_f64().unwrap() > 0.0, "{anchor} {dx}: eight cameras scroll in the room under the pill: {list}");
                    let (window, sheet) = (&list["window"], &list["sheet"]);
                    assert!(window["y"].as_f64().unwrap() + window["h"].as_f64().unwrap() <= sheet["y"].as_f64().unwrap() + sheet["h"].as_f64().unwrap(), "the column stays on the sheet: {list}");
                }
                key(&mut shell, "back", t);
                t += 600;
                shell.tick(t);
            }
        }
    }

    #[test]
    fn ok_on_the_cameras_card_opens_the_grid_in_every_link_state_and_back_returns_to_the_card() {
        let layout = json!({"grid": {"rows": 16}, "widgets": [
            {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 4, "rows": 4}, {"id": "cameras", "type": "cameras", "col": 0, "row": 4, "span": 4, "rows": 5},
            {"id": "news", "type": "news", "col": 4, "row": 0, "span": 5, "rows": 9}, {"id": "weather", "type": "weather", "col": 9, "row": 0, "span": 3, "rows": 9},
            {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        let camera = json!({"id": "front", "name": "Front door", "room": "Porch", "online": true, "live": true, "recordings": true, "events": true, "clips": false, "since": 0,
                            "native": {"codec": "h264", "width": 1920, "height": 1080, "fps": 15}, "prebuffered": true});
        // the owner's layout is a file in the data directory, which Home reads when it comes to the front
        let data = std::env::temp_dir().join(format!("lumatv-ui-cameras-card-{}", std::process::id()));
        std::fs::create_dir_all(data.join("theme")).unwrap();
        std::fs::write(data.join("theme/layout.json"), layout.to_string()).unwrap();
        for (link, cameras) in [("ready", vec![camera]), ("unlinked", vec![])] {
            let (_h, kit) = testkit::kit(1.0);
            let theme = lumatv_tokens::Tokens::shipped().json().clone();
            let mut shell = Shell::new(kit, Kinds::parse("home,cameras"), 2000, &testkit::module_dir().join("icons/audio"), &theme, &layout).expect("a shell");
            shell.dirs(&testkit::module_dir(), &data);
            shell.start();
            shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1_000);
            shell.helper(&Event::Topic { topic: "cameras".into(), data: json!({"state": link, "reason": "", "now_skew_ms": 0, "rev": 1, "cameras": cameras}) }, 1_100);
            shell.home.walk_to_card("cameras", 2_000);
            assert_eq!(shell.state()["home"]["widgetFocusId"], "cameras", "{link}");
            key(&mut shell, "ok", 2_400);
            assert_eq!(shell.state()["cameras"]["screen"]["mode"], "grid", "{link}");
            assert_eq!(shell.state()["cameras"]["link"], link);
            key(&mut shell, "back", 2_800);
            assert_eq!(shell.state()["cameras"]["screen"]["mode"], "", "{link}");
            assert_eq!(shell.state()["home"]["widgetFocusId"], "cameras", "Home is as it was left: {link}");
        }
        let _ = std::fs::remove_dir_all(&data);
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

    #[test]
    fn the_camera_pop_up_claims_a_video_window_first_asks_for_the_stream_when_it_is_granted_and_opens_its_hole_when_a_sink_has_it() {
        let (_h, mut shell) = home_shell("popup,cameras");
        let record = json!({"id": 41, "event": "e41", "camera": "275", "name": "Cat Feeder", "kind": "animal", "at": 1_000, "untilMs": u64::MAX / 4, "thumbnail": false, "live": true});
        let mut acts = shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record }, 2_000);
        acts.extend(shell.tick(2_050));
        let posted = claims(&acts);
        assert_eq!(posted.len(), 1, "one claim for the picture box: {posted:?}");
        assert_eq!((posted[0]["op"].as_str(), posted[0]["kind"].as_str(), posted[0]["id"].as_str()), (Some("claim"), Some("camera-popup"), Some("camera-popup")));
        assert_eq!(posted[0]["rect"]["w"], 520, "the card's picture box: {posted:?}");
        assert_eq!(helper_calls(&acts, "/v1/cameras/live"), 0, "no stream before a window is granted");

        // granted and not drawing yet: the helper is asked, for the window the compositor named
        let line = |placed: bool, sink: Value| json!({"id": "camera-popup", "kind": "camera-popup", "state": "granted", "reason": "", "text": "", "slot": "main", "placed": placed, "sink": sink});
        let topic = |claim: Value| Event::Topic { topic: "vwin".into(), data: json!({"mode": "idle", "slots": 2, "free": 1, "apps": 0, "wall": false, "claims": [claim]}) };
        let acts = shell.channel(&topic(line(false, Value::Null)), 2_100);
        assert_eq!(helper_calls(&acts, "/v1/cameras/live"), 1, "asked once the window is granted");
        let asked = acts
            .iter()
            .find_map(|a| match a {
                Act::Helper(o) if o.path == "/v1/cameras/live" => o.body.clone(),
                _ => None,
            })
            .unwrap();
        assert_eq!(asked, json!({"camera": "275", "for": "popup", "window": "main", "popup": 41}));

        // the stream plays and the window draws, but nothing lists the video on a sink yet: no hole
        shell.helper(&Event::Topic { topic: "cameras.live".into(), data: json!({"camera": "275", "mode": "live", "state": "playing", "fps": 15, "fedFps": 14.8}) }, 2_600);
        shell.channel(&topic(line(true, Value::Null)), 2_700);
        shell.tick(3_100);
        assert_eq!(shell.state()["popup"]["live"]["video"], false, "a hole over a decoder nobody has seen connected is black");
        // a sink has it: the hole opens once the handoff has passed
        shell.channel(&topic(line(true, json!("MAIN"))), 3_200);
        shell.tick(3_300);
        assert_eq!(shell.state()["popup"]["live"]["video"], true, "{}", shell.state()["popup"]);
        assert_eq!(shell.geometry()["popup"]["video"], true);

        // Back: the card goes, the claim is released, the stream is stopped
        let acts = key(&mut shell, "back", 4_000);
        let posted = claims(&acts);
        assert_eq!(posted.iter().map(|b| b["op"].as_str().unwrap_or("")).collect::<Vec<_>>(), vec!["release"], "{posted:?}");
        assert_eq!(helper_calls(&acts, "/v1/cameras/live/stop"), 1);
    }

    #[test]
    fn a_card_that_replaces_the_one_up_keeps_the_claim_the_stream_is_played_in() {
        let (_h, mut shell) = home_shell("popup,cameras");
        let record = |id: u64| json!({"id": id, "event": format!("e{id}"), "camera": "275", "name": "Cat Feeder", "kind": "person", "at": 1_000, "untilMs": u64::MAX / 4, "thumbnail": false, "live": true});
        let line = json!({"id": "camera-popup", "kind": "camera-popup", "state": "granted", "reason": "", "text": "", "slot": "texture", "placed": true, "sink": null});
        let topic = |claim: Value| Event::Topic { topic: "vwin".into(), data: json!({"mode": "idle", "slots": 2, "free": 1, "apps": 0, "wall": false, "claims": [claim]}) };
        let mut acts = shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record(41) }, 2_000);
        acts.extend(shell.tick(2_050));
        shell.channel(&topic(line.clone()), 2_100);
        shell.helper(&Event::Topic { topic: "cameras.live".into(), data: json!({"camera": "275", "mode": "live", "state": "playing", "fps": 15, "fedFps": 14.8}) }, 2_600);
        shell.tick(3_000);
        assert_eq!(shell.state()["popup"]["live"]["video"], true, "{}", shell.state()["popup"]);

        // another event of the same camera: the card is replaced, the old stream is stopped and the new one asked for. The helper's topic says the old
        // stream's last words first (purpose `pop-up 41`), then the new one's: none of the first is the new card's
        let live = |state: &str, purpose: &str| Event::Topic { topic: "cameras.live".into(), data: json!({"camera": "275", "mode": "live", "state": state, "purpose": purpose, "fps": 15, "fedFps": 14.8}) };
        shell.helper(&live("playing", "pop-up 41"), 2_700);
        let mut acts = shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record(42) }, 4_000);
        acts.extend(shell.tick(4_050));
        acts.extend(shell.channel(&topic(line.clone()), 4_100));
        assert_eq!(helper_calls(&acts, "/v1/cameras/live"), 1, "the new card asks for its stream");
        acts.extend(shell.helper(&live("playing", "pop-up 41"), 4_120));
        acts.extend(shell.helper(&live("stopped", "pop-up 41"), 4_150));
        acts.extend(shell.tick(4_160));
        assert_eq!(shell.state()["popup"]["live"]["phase"], "asked", "{}", shell.state()["popup"]["live"]);
        acts.extend(shell.helper(&live("starting", "pop-up 42"), 4_200));
        acts.extend(shell.helper(&live("playing", "pop-up 42"), 4_300));
        acts.extend(shell.tick(4_700));
        let ops: Vec<String> = claims(&acts).iter().map(|b| format!("{} {}", b["op"], b["id"])).collect();
        assert!(ops.is_empty(), "the claim is neither released nor posted again: {ops:?}");
        assert_eq!(helper_calls(&acts, "/v1/cameras/live/stop"), 1, "only the old stream's stop: {acts:?}");
        assert_eq!(shell.state()["popup"]["live"]["video"], true, "{}", shell.state()["popup"]);
    }

    #[test]
    fn the_camera_pop_ups_video_window_is_where_the_owner_placed_the_card_and_in_its_corner_half_of_homes_safe_margin_in_by_default() {
        let record = json!({"id": 42, "event": "e42", "camera": "275", "name": "Cat Feeder", "kind": "person", "at": 1_000, "untilMs": u64::MAX / 4, "thumbnail": false, "live": true});
        let claim_of = |shell: &mut Shell| {
            let mut acts = shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record.clone() }, 2_000);
            acts.extend(shell.tick(2_050));
            claims(&acts).into_iter().find(|c| c["id"] == "camera-popup").expect("the card claims a window")["rect"].clone()
        };
        let (_h, mut shell) = home_shell("popup,cameras");
        let r = claim_of(&mut shell);
        // the card is 560 wide with 20 px of air round the 520 wide picture: its right edge is at 1888, 32 from the screen's, and its top at 32
        assert_eq!((r["x"].as_i64().unwrap() + r["w"].as_i64().unwrap() + 20, r["y"].as_i64().unwrap() < 32 + 160), (1888, true), "{r}");
        let placed = json!({"layout": {"overlays": {"popup": {"anchor": "bottom-left", "dx": 0, "dy": 0}}}});
        let (_h, mut moved) = home_shell_with("popup,cameras", &placed);
        let r = claim_of(&mut moved);
        assert_eq!(r["x"].as_i64().unwrap(), 32 + 20, "the picture's window follows the card to the left edge of its zone: {r}");
        assert!(r["y"].as_i64().unwrap() + r["h"].as_i64().unwrap() <= 810, "and stays above the bottom quarter: {r}");
    }

    #[test]
    fn ok_on_a_live_card_over_an_app_brings_home_forward_for_the_full_view_which_asks_for_its_stream_when_home_is_in_front_and_back_brings_the_app_back() {
        let (_h, mut shell) = home_shell("popup,cameras");
        let camera = json!({"id": "275", "name": "Cat Feeder", "room": "", "online": true, "live": true, "recordings": true, "events": true, "clips": false, "since": 0,
                            "native": {"codec": "h264", "width": 3840, "height": 2160, "fps": 15}, "prebuffered": true});
        shell.helper(&Event::Topic { topic: "cameras".into(), data: json!({"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1, "cameras": [camera]}) }, 1_100);
        shell.channel(&Event::Topic { topic: "vwin".into(), data: json!({"mode": "idle", "slots": 2, "free": 2, "apps": 0, "wall": false, "claims": []}) }, 1_150);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false, "covered": true}) }, 1_200);
        let record = json!({"id": 41, "event": "e41", "camera": "275", "name": "Cat Feeder", "kind": "person", "at": 1_000, "untilMs": u64::MAX / 4, "thumbnail": false, "live": true});
        shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record }, 2_000);
        shell.tick(2_050);
        assert_eq!(shell.state()["popup"]["up"], true, "the card is over the app");

        // OK, once the card has been up 600 ms: the view opens at once, Home comes forward, and nothing is decoded beside the app's video
        let acts = key(&mut shell, "ok", 2_800);
        assert!(shell.cameras().app.viewer_up(), "the view is up in the first frame");
        let launches = luna_calls(&acts, "/launchDefaultApp");
        assert!(launches.iter().any(|l| l["payload"]["category"] == "home"), "Home comes forward: {launches:?}");
        assert_eq!(helper_calls(&acts, "/v1/cameras/live"), 0, "no decoder while the app is in front");

        // Home is in front: the camera's own stream is asked for, for the main window
        let acts = shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 3_000);
        let asked: Vec<Value> = acts
            .iter()
            .filter_map(|a| match a {
                Act::Helper(o) if o.path == "/v1/cameras/live" => o.body.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(asked, vec![json!({"camera": "275", "for": "full", "window": "main"})]);

        // Back: the view is gone and the app is raised by the compositor, not launched again (a launch reloads YouTube at its start page)
        let acts = key(&mut shell, "back", 5_000);
        assert!(!shell.cameras().app.viewer_up());
        let fronts: Vec<Value> = acts
            .iter()
            .filter_map(|a| match a {
                Act::Channel(o) if o.path == "/v1/front" => o.body.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(fronts, vec![json!({"app": "netflix"})], "the app comes back");
        assert!(luna_calls(&acts, "/launch").is_empty(), "and is not launched");
        assert_eq!(helper_calls(&acts, "/v1/cameras/live/stop"), 1);
    }

    #[test]
    fn ok_on_a_live_card_over_an_app_with_a_video_stays_over_it_pauses_it_asks_for_the_sub_window_then_the_stream_and_back_plays_it_again() {
        let (_h, mut shell) = home_shell("popup,cameras");
        let camera = json!({"id": "275", "name": "Cat Feeder", "room": "", "online": true, "live": true, "recordings": true, "events": true, "clips": false, "since": 0,
                            "native": {"codec": "h264", "width": 3840, "height": 2160, "fps": 15}, "prebuffered": true});
        shell.helper(&Event::Topic { topic: "cameras".into(), data: json!({"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1, "cameras": [camera]}) }, 1_100);
        shell.channel(&Event::Topic { topic: "vwin".into(), data: json!({"mode": "apps", "slots": 2, "free": 1, "apps": 1, "appsPlaying": 1, "wall": false, "claims": []}) }, 1_150);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "youtube.leanback.v4", "home": false, "covered": true}) }, 1_200);
        let record = json!({"id": 41, "event": "e41", "camera": "275", "name": "Cat Feeder", "kind": "person", "at": 1_000, "untilMs": u64::MAX / 4, "thumbnail": false, "live": true});
        shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record }, 2_000);
        shell.tick(2_050);

        // OK: the view opens over the app, which is paused by its own key; Home stays where it is and the helper is not asked yet
        let acts = key(&mut shell, "ok", 2_800);
        assert!(shell.cameras().app.viewer_up());
        assert!(luna_calls(&acts, "/launchDefaultApp").is_empty(), "the app is not sent to the background");
        let keys = |acts: &[Act]| -> Vec<Value> { acts.iter().filter_map(|a| match a { Act::Channel(o) if o.path == "/v1/appkey" => o.body.clone(), _ => None }).collect() };
        assert!(keys(&acts).is_empty(), "nothing is paused before the media controller says the video plays");
        assert_eq!(luna_calls(&acts, "/getMediaSessionId").len(), 1, "it is asked whether the app's video plays");
        let said = |shell: &mut Shell, tag: u64, payload: Value, at: u64| shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": tag, "payload": payload}) }, at);
        said(&mut shell, crate::cameras::app::TAG_SESSIONS, json!({"returnValue": true, "mediaId": ["XCFRHKH"]}), 2_850);
        let acts = said(&mut shell, crate::cameras::app::TAG_STATUS, json!({"returnValue": true, "playStatus": "PLAYSTATE_PLAYING"}), 2_900);
        assert_eq!(keys(&acts), vec![json!({"app": "youtube.leanback.v4", "key": "pause"})]);
        assert_eq!(helper_calls(&acts, "/v1/cameras/live"), 0, "the stream waits for the window");
        let claims = shell.state()["vwin"]["claims"].clone();
        assert!(claims["cameras-full"].is_object(), "the claim for the sub window is made first: {claims}");

        // the compositor grants the sub window: the stream is asked for, in it
        let line = json!({"id": "cameras-full", "kind": "camera-full", "state": "granted", "reason": "", "text": "", "slot": "sub", "placed": false});
        let acts = shell.channel(&Event::Topic { topic: "vwin".into(), data: json!({"mode": "apps", "slots": 2, "free": 0, "apps": 1, "appsPlaying": 1, "wall": false, "claims": [line]}) }, 3_000);
        let asked: Vec<Value> = acts.iter().filter_map(|a| match a { Act::Helper(o) if o.path == "/v1/cameras/live" => o.body.clone(), _ => None }).collect();
        assert_eq!(asked, vec![json!({"camera": "275", "for": "full", "window": "sub"})]);

        // Back: the app's video plays again, and the app is neither raised nor launched (it never left the front)
        let acts = key(&mut shell, "back", 5_000);
        assert!(!shell.cameras().app.viewer_up());
        assert_eq!(keys(&acts), vec![json!({"app": "youtube.leanback.v4", "key": "play"})]);
        assert!(acts.iter().all(|a| !matches!(a, Act::Channel(o) if o.path == "/v1/front")));
        assert!(luna_calls(&acts, "/launch").is_empty());
        assert_eq!(helper_calls(&acts, "/v1/cameras/live/stop"), 1);
    }

    #[test]
    fn ok_held_on_the_camera_card_opens_the_snooze_sheet_whose_pick_snoozes_the_camera_ends_the_card_and_says_so() {
        let (_h, mut shell) = home_shell("popup,menu");
        let record = json!({"id": 41, "event": "e41", "camera": "275", "name": "Cat Feeder", "kind": "person", "at": 1_000, "untilMs": u64::MAX / 4, "thumbnail": false});
        shell.helper(&Event::Topic { topic: "cameras.popup".into(), data: record }, 2_000);
        shell.tick(2_050);
        let ok = |down| KeyEvent::Key { name: "ok".into(), down, repeat: false, seat: 0 };
        shell.key(&ok(true), 2_800);
        shell.tick(2_800 + crate::panels::layer::HOLD_MS);
        assert_eq!(shell.panels.up(&shell.kinds), Some(Which::Menu), "the sheet");
        shell.key(&ok(false), 3_600);
        shell.tick(3_700);
        assert_eq!((shell.panels.up(&shell.kinds), shell.state()["popup"]["up"].clone()), (Some(Which::Menu), json!(false)), "the release is nothing, and the card steps aside");
        // OK goes on to the time, and OK on it snoozes this camera for an hour
        key(&mut shell, "ok", 4_000);
        let acts = key(&mut shell, "ok", 4_200);
        let asked: Vec<Value> = acts.iter().filter_map(|a| match a { Act::Helper(o) if o.path == "/v1/dnd/snooze" => o.body.clone(), _ => None }).collect();
        assert_eq!(asked, vec![json!({"minutes": 60, "kind": "camera", "label": "Cat Feeder", "source": "275"})]);
        assert_eq!(helper_calls(&acts, "/v1/cameras/popup/clear"), 1);
        assert_eq!(luna_calls(&acts, "/createToast")[0]["payload"]["message"], "Cat Feeder snoozed for 1 hour");
        shell.tick(5_000);
        assert_eq!(shell.state()["popup"]["up"], false, "the card does not come back");
    }

    /// Opens the kind `name` over Home; the kinds that nothing but their owner closes are closed by `close_owner`.
    fn open_owner(shell: &mut Shell, name: &str, at: u64) {
        let mut acts = Vec::new();
        let kinds = shell.kinds.clone();
        let mut out = Vec::new();
        match name {
            "settings" => {
                shell.open_settings(None, at);
            }
            "quick" | "picker" | "drawer" => {
                shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": name, "n": at}) }, at);
            }
            "menu" => {
                shell.channel(&Event::Topic { topic: "menu.open".into(), data: json!({"items": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]}) }, at);
            }
            "confirm" => {
                shell.channel(&Event::Topic { topic: "confirm.ask".into(), data: json!({"title": "Sure?", "answers": [{"id": "no", "label": "No"}, {"id": "yes", "label": "Yes"}]}) }, at);
            }
            "entry" => {
                shell.panels.ask_text("Name", "", false, &kinds, at, &mut out);
            }
            "keyboard" => {
                shell.channel(&Event::Topic { topic: "ime".into(), data: json!({"active": true, "show": true, "model": 1}) }, at);
            }
            "layout" => {
                shell.open_layout(None, at);
            }
            "cameras" => {
                out.extend(shell.cameras().app.open_full("273", at));
            }
            "ai" => {
                let doc = json!({"id": "a1", "kind": "tv_standby", "assistant": {"id": "c", "name": "Cody"}, "title": "Turn the TV off", "detail": "The TV goes to standby.",
                                 "reason": "Nitin said good night", "shownAt": 1_000, "expiresAt": 90_000_000, "allow": "OK", "deny": "Back"});
                shell.helper(&Event::Topic { topic: "ai.prompt".into(), data: doc }, at);
            }
            other => panic!("no way to open {other}"),
        }
        for e in out {
            shell.carry(e, at, &mut acts);
        }
        shell.tick(at + 10);
    }

    fn close_owner(shell: &mut Shell, name: &str, at: u64) {
        let mut acts = Vec::new();
        match name {
            "keyboard" => {
                shell.channel(&Event::Topic { topic: "ime".into(), data: json!({"active": false, "show": false, "model": 1}) }, at);
            }
            "ai" => {
                shell.helper(&Event::Topic { topic: "ai.prompt".into(), data: Value::Null }, at);
            }
            "cameras" => {
                for e in shell.cameras().app.close(at) {
                    shell.carry(e, at, &mut acts);
                }
            }
            _ => {}
        }
        // what the person's own clicks opened over the kind (the Cameras grid opens Settings) goes with Back
        for i in 0..8 {
            if !shell.system_wants() {
                break;
            }
            key(shell, "back", at + 100 + i * 100);
        }
        shell.tick(at + 1_000);
    }

    /// What Home shows of where the focus is and what it launched: it must not move while another kind owns input.
    fn home_position(shell: &Shell) -> Value {
        let h = shell.state()["home"].clone();
        json!({"focus": h["focus"], "dock": h["dockIndex"], "layer": h["layer"], "launching": h["launching"], "moving": h["dockMoving"]})
    }

    #[test]
    fn while_a_kind_above_home_owns_input_home_gets_no_key_ok_or_pointer_and_gets_them_again_when_it_closes() {
        for name in ["settings", "quick", "picker", "drawer", "menu", "confirm", "entry", "keyboard", "layout", "cameras", "ai"] {
            let (_h, mut shell) = home_shell("home,settings,cameras,quick,picker,drawer,menu,confirm,entry,keyboard,layout,ai");
            open_owner(&mut shell, name, 2_000);
            assert!(shell.system_wants(), "{name}: it owns input");
            let mut at = 2_100;
            let mut launched = 0;
            // every input that arrives while the kind owns input leaves Home where it was
            let mut send = |shell: &mut Shell, event: KeyEvent, at: u64, what: &str| {
                let owned = shell.system_wants();
                let before = home_position(shell);
                let acts = shell.key(&event, at);
                if owned {
                    assert_eq!(home_position(shell), before, "{name}: {what} moved Home");
                    // an app of Home's dock or library: the kinds' own launches (the picker's inputs) are not Home's
                    launched += luna_calls(&acts, "/launch").iter().filter(|c| ["netflix", "youtube", "plex", "extra.a", "extra.b", "com.webos.app.discovery"].contains(&c["payload"]["id"].as_str().unwrap_or(""))).count();
                }
            };
            for k in ["right", "right", "down", "left", "up", "3", "ok"] {
                for down in [true, false] {
                    send(&mut shell, KeyEvent::Key { name: k.into(), down, repeat: false, seat: 0 }, at, k);
                    at += 50;
                }
            }
            // clicks over the whole screen, the dock's row among them
            for y in [140.0, 400.0, 700.0, 960.0, 1040.0] {
                for x in [100.0, 400.0, 700.0, 960.0, 1300.0, 1600.0, 1850.0] {
                    for buttons in [0, 0, 1, 0] {
                        send(&mut shell, KeyEvent::Pointer { x, y, buttons, visible: true }, at, "a pointer sample");
                        at += 20;
                    }
                }
            }
            assert_eq!(launched, 0, "{name}: Home launched nothing");
            close_owner(&mut shell, name, at);
            assert!(!shell.system_wants(), "{name}: closed");
            // an owner that closed itself (the picker on OK) gave Home the clicks that followed, and a click on the weather card opens its view: Back leaves it
            for i in 0..3 {
                if shell.state()["home"]["layer"] != "" {
                    key(&mut shell, "back", at + 200 + i * 100);
                }
            }
            let before = home_position(&shell);
            // (the focus may be on the rightmost card, which a click left it on: then the way back is left)
            key(&mut shell, "right", at + 1_000);
            if home_position(&shell) == before {
                key(&mut shell, "left", at + 1_200);
            }
            assert_ne!(home_position(&shell), before, "{name}: Home has the keys again");
        }
    }

    #[test]
    fn the_mute_mark_keeps_to_the_left_while_the_quick_panel_is_up() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("osd,quick"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        shell.start();
        shell.tick(1_000);
        assert_eq!(shell.volume.view().spots, crate::system::volume::mark_spots(false, false), "no panel: every corner");
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "quick", "n": 1}) }, 1_100);
        shell.tick(1_200);
        assert_eq!(shell.volume.view().spots, crate::system::volume::mark_spots(false, true), "the panel is up on the right: the mark's corners are the left ones");
    }

    #[test]
    fn a_home_behind_an_app_asks_the_engine_for_nothing_as_the_seconds_pass_and_is_right_when_it_returns() {
        // The analogue clock's second hand, the news slide and the poses sent again each cost the compositor a composed frame a second under a
        // playing video (round 19, YouTube: 1 to 2 page flips a second with nothing over it, each a 12 ms GPU frame the hole wipes again).
        // (the harness's own `Commit` marker ends every read: a request is anything else)
        let sent = |h: &mut ui::testing::Harness| h.commit().into_iter().filter(|r| !format!("{r:?}").starts_with("Commit")).collect::<Vec<_>>();
        let (mut h, mut shell) = home_shell_with("home", &json!({"clock": {"style": "analog"}}));
        shell.tick(1_100);
        sent(&mut h);
        shell.tick(2_100);
        assert!(!sent(&mut h).is_empty(), "in front, the second hand moves: the second sends its picture");
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "youtube.leanback.v4", "home": false, "covered": true}) }, 2_500);
        sent(&mut h);
        for s in 3..40u64 {
            let acts = shell.tick(s * 1000 + 50);
            let reqs = sent(&mut h);
            assert!(reqs.is_empty(), "t={s} s covered: {} requests, first {:?}", reqs.len(), reqs.first());
            assert!(acts.is_empty(), "t={s} s covered: {acts:?}");
        }
        assert!(shell.due_at().is_none_or(|due| due > 40_000), "a covered Home wakes the client for no second: next due {:?}", shell.due_at());
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 41_000);
        shell.tick(41_050);
        assert!(!sent(&mut h).is_empty(), "in front again, the hands are drawn for the time it is");
        assert!(shell.due_at().is_some_and(|due| due <= 42_100), "and the second hand is due again: {:?}", shell.due_at());
    }

    #[test]
    fn the_weather_picture_asks_the_engine_for_a_tick_only_while_its_card_is_seen() {
        let reading = |code: i64| json!({"doc": {"fetchedAt": 5, "current": {"code": code, "isDay": true, "windMph": 4.0}, "display": {"place": "Atlanta", "temp": "61\u{b0}", "cond": "Rain", "stats": "", "nowKind": "rain", "days": []}}});
        let (mut h, mut shell) = home_shell("home");
        shell.helper(&Event::Topic { topic: "weather".into(), data: reading(63) }, 1_050);
        shell.tick(1_100);
        // the picture is made once the card's words are on the atlas (their glyphs do not wait behind its pixels)
        h.settle_atlas();
        shell.tick(1_150);
        let due = shell.due_at().expect("rain moves: a tick is due");
        assert!(due <= 1_100 + crate::home::weather_art::TICK_MS, "the paths are restated every {} ms, not {}", crate::home::weather_art::TICK_MS, due - 1_100);
        // a tick moves the next due on, and nothing else is asked
        shell.tick(due);
        assert!(shell.due_at().is_some_and(|d| d > due && d <= due + crate::home::weather_art::TICK_MS));
        // the screen off, a screensaver, reduced motion and an app over Home: no tick, nothing is due for the picture
        shell.channel(&Event::Topic { topic: "screen".into(), data: json!({"on": false}) }, 3_000);
        shell.tick(3_050);
        assert!(shell.due_at().is_none_or(|d| d > 3_050 + crate::home::weather_art::TICK_MS), "screen off: {:?}", shell.due_at());
        shell.channel(&Event::Topic { topic: "screen".into(), data: json!({"on": true}) }, 4_000);
        shell.tick(4_050);
        assert!(shell.due_at().is_some_and(|d| d <= 4_050 + crate::home::weather_art::TICK_MS), "back on: it moves again");
        shell.kit.ui.look.reduced_motion = true;
        shell.tick(4_500);
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "youtube.leanback.v4", "home": false, "covered": true}) }, 5_000);
        for s in 6..14u64 {
            shell.tick(s * 1000 + 50);
        }
        assert_eq!(shell.due_at(), None, "reduced motion and covered: nothing is due");
        shell.kit.ui.look.reduced_motion = false;
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "youtube.leanback.v4", "home": false, "covered": true}) }, 15_000);
        shell.tick(15_050);
        assert_eq!(shell.due_at(), None, "an app over Home: the picture asks for nothing");
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 16_000);
        shell.tick(16_050);
        assert!(shell.due_at().is_some_and(|d| d <= 16_050 + crate::home::weather_art::TICK_MS), "Home in front again: it moves");
    }

    /// The sky restates its paths every 0.4 s, and at rest that was most of the shell's time: a whole apply of Home (0.6 ms of the host's thread, a
    /// glass, a status bar and five icons sent again to be found the same) and the report built to be found the same. A made sky's step sends its own
    /// paths and nothing else; the clock's second still draws Home.
    #[test]
    fn a_made_skys_restate_sends_its_paths_alone_and_builds_no_report() {
        let reading = |temp: &str, at: u64| json!({"doc": {"fetchedAt": at, "current": {"code": 63, "isDay": true, "windMph": 4.0}, "display": {"place": "Atlanta", "temp": temp, "cond": "Rain", "stats": "", "nowKind": "rain", "days": []}}});
        let sent = |h: &mut ui::testing::Harness| h.commit().into_iter().filter(|r| !format!("{r:?}").starts_with("Commit")).collect::<Vec<_>>();
        let paths_only = |reqs: &[compositor_protocol::Request]| !reqs.is_empty() && reqs.iter().all(|r| matches!(r, compositor_protocol::Request::Animate { .. } | compositor_protocol::Request::Set { .. }));
        let (mut h, mut shell) = home_shell("home");
        shell.helper(&Event::Topic { topic: "weather".into(), data: reading("61\u{b0}", 5) }, 1_050);
        shell.tick(1_100);
        h.settle_atlas();
        // the sky not made yet: its first step is the apply's
        shell.tick(1_150);
        let due = shell.due_at().expect("rain moves: a tick is due");
        assert!(due < 1_600, "the step is in the second the clock was drawn in: {due}");
        sent(&mut h);
        let built = shell.reports_built;
        shell.tick(due);
        let reqs = sent(&mut h);
        assert!(paths_only(&reqs), "the step's own paths: {reqs:?}");
        assert_eq!(shell.reports_built, built, "nothing changed: no report is built");
        // words with glyphs the atlas has not seen leave it unsettled, and a sky that is made does not wait for it
        shell.helper(&Event::Topic { topic: "weather".into(), data: reading("QZ\u{b0}", 6) }, due + 10);
        assert!(shell.kit.ui.text_atlas().is_some_and(|a| !a.is_settled()), "the new words are not on the atlas yet");
        let next = shell.due_at().expect("rain moves: a tick is due");
        assert!(next < 2_000, "the step is in the same second: {next}");
        sent(&mut h);
        let built = shell.reports_built;
        shell.tick(next);
        let reqs = sent(&mut h);
        assert!(paths_only(&reqs), "the step's own paths: {reqs:?}");
        assert_eq!(shell.reports_built, built, "nothing changed: no report is built");
        // a second goes by and the clock's hands are drawn: Home is applied
        shell.tick(2_050);
        assert!(shell.reports_built > built, "the report is looked at again within the second");
    }

    #[test]
    fn with_every_kind_built_and_an_app_in_front_nothing_of_the_shell_is_due() {
        // A timer that is due behind a playing video wakes the client and, if it changes a picture, costs a composed frame under it: the shell with
        // every kind and Home covered sleeps until an event.
        let kinds = "home,osd,settings,toast,cameras,quick,picker,drawer,menu,confirm,switcher,entry,keyboard,alert,saver,capture,cards,popup,layout,ai,banner,displaylab";
        let (_h, mut shell) = home_shell(kinds);
        shell.tick(1_100);
        // tvpower takes the saver's vote, which stays while the app is in front
        for tag in [0x5359_0010u64, 0x5359_0011] {
            shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": tag, "payload": {"returnValue": true, "subscribed": true}}) }, 1_200);
        }
        shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "youtube.leanback.v4", "home": false, "covered": true}) }, 2_500);
        for s in 3..6u64 {
            shell.tick(s * 1000 + 50);
        }
        assert_eq!(shell.due_at(), None, "nothing is due while an app is in front");
    }

    #[test]
    fn a_toast_is_raised_by_the_show_topic_through_the_feeds_own_path() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("osd,toast"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        shell.start();
        assert!(shell.report()["can"].as_array().unwrap().iter().any(|c| c == "toast"), "the compositor may ask for one by name");
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "toast", "warm": false, "n": 1, "page": "Placement_check|Second_line"}) }, 1_000);
        assert_eq!(shell.state()["toasts"], 1);
        assert!(shell.is_live(), "and it is drawn");
        assert_eq!(shell.geometry()["toasts"].as_array().unwrap().len(), 1);
        shell.tick(8_900);
        assert_eq!(shell.state()["toasts"], 1, "it stays up for 8 s");
        shell.tick(9_100);
        assert_eq!(shell.state()["toasts"], 0);
        // no words: a toast of the default ones
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "toast", "n": 2}) }, 10_000);
        assert_eq!(shell.state()["toasts"], 1);
        // a process without the kind says it cannot
        let (_h2, kit) = testkit::kit(1.0);
        let mut other = Shell::new(kit, Kinds::parse("osd"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        other.start();
        assert!(!other.report()["can"].as_array().unwrap().iter().any(|c| c == "toast"));
    }

    #[test]
    fn the_format_badge_is_raised_by_the_show_topic_in_the_top_right_corner_and_leaves_after_three_seconds() {
        let (_h, kit) = testkit::kit(1.0);
        let mut shell = Shell::new(kit, Kinds::parse("osd,toast"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        shell.start();
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "toast", "n": 1, "page": "format:uhd,dolby-vision,atmos"}) }, 1_000);
        assert_eq!((shell.state()["format"].clone(), shell.state()["toasts"].clone()), (json!(3), json!(0)), "three marks, and no toast");
        assert!(shell.is_live(), "and it is drawn");
        let at = shell.geometry()["format"].as_array().unwrap().clone();
        let edge = |k: &str| at[0][k].as_f64().unwrap();
        assert_eq!((at.len(), edge("x") + edge("w"), edge("y")), (1, 1880.0, 40.0), "40 px in from the top right corner: {at:?}");
        shell.tick(3_900);
        assert_eq!(shell.state()["format"], 3, "it stays up for 3 s");
        shell.tick(4_100);
        assert_eq!(shell.state()["format"], 0);
        assert!(shell.geometry()["format"].as_array().unwrap().is_empty());
    }

    /// A TV's Home over a full dock holds 64 of the scene's 96 groups, and the gear's panel, closed a moment ago, is kept built for four seconds
    /// more: Settings opened from it took what was left, and the rows that did not fit were not drawn ("at most 96 groups", over and over).
    #[test]
    fn settings_opening_over_a_full_home_takes_the_parked_gear_panel_away_and_draws_the_whole_page() {
        let (_h, mut shell) = home_shell("quick,settings");
        let used = 96 - shell.kit.room().groups;
        let scene = shell.kit.ui.scene.clone();
        let _full_dock: Vec<_> = (0..64usize.saturating_sub(used)).map(|_| scene.group(None, compositor_protocol::GroupPose::IDENTITY).expect("a group")).collect();
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "quick", "warm": false, "n": 1}) }, 2_000);
        shell.tick(2_300);
        assert_eq!(shell.state()["panels"]["quick"], "open");
        key(&mut shell, "back", 2_400);
        shell.tick(2_600);
        assert_eq!(shell.state()["panels"]["quick"], "parked", "closed a moment ago: its layers are kept for the next opening");
        let acts = shell.open_settings(Some("inputs"), 2_700);
        assert_eq!(shell.state()["panels"]["quick"], "gone", "Settings takes the room at once");
        let logs: Vec<&String> = acts.iter().filter_map(|a| if let Act::Log(l) = a { Some(l) } else { None }).collect();
        assert!(!logs.iter().any(|l| l.contains("groups") || l.contains("scene is full")), "{logs:?}");
        assert_eq!(shell.state()["settings"]["built"], 7, "every row of the page is drawn");
        assert!(shell.kit.room().groups >= 4, "{:?}", shell.kit.room());
    }

    /// 2026-11-07 18:27 on the set: the gear's tap opened the quick panel and its long press 1.6 s later opened Settings through the compositor's
    /// `show`. The panel stayed open under Settings with its layers and groups, Settings ran out of the scene ("at most 96 groups", the cards
    /// "scene is full") and its focus plate and profile chip went until the self-heal made them again; Back then closed Settings and the panel.
    #[test]
    fn settings_shown_over_the_open_quick_panel_closes_the_panel_and_draws_the_whole_page() {
        let (_h, mut shell) = home_shell("quick,settings");
        let used = 96 - shell.kit.room().groups;
        let scene = shell.kit.ui.scene.clone();
        let _full_dock: Vec<_> = (0..64usize.saturating_sub(used)).map(|_| scene.group(None, compositor_protocol::GroupPose::IDENTITY).expect("a group")).collect();
        shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "quick", "warm": false, "n": 1}) }, 2_000);
        shell.tick(2_300);
        assert_eq!(shell.state()["panels"]["quick"], "open");
        let mut acts = shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "settings", "page": "inputs", "n": 2}) }, 3_600);
        acts.extend(shell.tick(3_610));
        assert!(matches!(shell.state()["panels"]["quick"].as_str(), Some("leaving" | "gone")), "{}", shell.state()["panels"]);
        acts.extend(shell.tick(4_200));
        let logs: Vec<&String> = acts.iter().filter_map(|a| if let Act::Log(l) = a { Some(l) } else { None }).collect();
        assert!(!logs.iter().any(|l| l.contains("groups") || l.contains("scene is full")), "{logs:?}");
        assert_eq!(shell.state()["settings"]["built"], 7, "every row of the page is drawn");
    }

    /// Session 11's settings-scroll at 120 Hz: Settings opened from the quick panel came up at once, the panel's slide out went on under it for
    /// 110 ms, and the dim behind the panel went at the slide's end in a frame of the whole screen, a refresh late. With no fade the panel goes
    /// as Settings comes; under the fade of 60 Hz it slides out as before.
    #[test]
    fn a_panel_sliding_out_goes_at_once_under_a_settings_that_comes_up_at_once() {
        for (hz, want) in [(120, "gone"), (60, "leaving")] {
            let (_h, mut shell) = home_shell("quick,settings");
            shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": hz}) }, 1_000);
            shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "quick", "warm": false, "n": 1}) }, 2_000);
            shell.tick(2_300);
            key(&mut shell, "back", 2_400);
            assert_eq!(shell.state()["panels"]["quick"], "leaving");
            shell.open_settings(Some("inputs"), 2_410);
            assert_eq!(shell.state()["panels"]["quick"], want, "{hz} Hz");
        }
    }

    /// Round 22's ring at 120 Hz: under Settings Home's GPU wallpaper kept stepping ten times a second and each step owed the whole screen, three
    /// frames of 157 layers; the weather's sprites drew a frame every refresh under the panes.
    #[test]
    fn home_is_calm_under_an_overlay_and_moves_again_when_it_has_been_gone_a_while() {
        let (_h, mut shell) = home_shell("home,quick,settings");
        shell.tick(1_500);
        assert!(!shell.home.wallpaper_held() && shell.home.weather_art_runs(), "Home alone moves");
        shell.open_settings(Some("inputs"), 2_000);
        shell.tick(2_050);
        assert!(shell.home.wallpaper_held() && !shell.home.weather_art_runs(), "Settings is up: the wallpaper stands and the weather rests");
        shell.tick(4_000);
        assert!(shell.home.wallpaper_held(), "however long it stays up");
        assert_eq!(shell.release_at, None, "and nothing is due for a release while it is");
        let _ = shell.settings.close(5_000);
        shell.tick(5_050);
        assert!(shell.home.wallpaper_held(), "its fade-out is still running: Home stays calm");
        let due = shell.release_at.expect("the release is a tick of its own");
        assert_eq!(due, 5_050 + OVERLAY_RELEASE_MS);
        assert!(shell.due_at().is_some_and(|d| d <= due));
        shell.tick(due - 1);
        assert!(shell.home.wallpaper_held());
        shell.tick(due);
        assert!(!shell.home.wallpaper_held() && shell.home.weather_art_runs(), "the overlay has been gone for a while: Home moves again");
        assert_eq!(shell.release_at, None);
    }

    /// A full-screen modal's fade redraws 157 layers a frame, 15-32 ms of GPU at the set: a refresh and a half at 60 Hz, and at 120 a fade of 30
    /// frames a second. From 100 Hz (the compositor's `refresh` topic) Settings opens and closes without it.
    #[test]
    fn the_refresh_topic_takes_the_modals_fade_away_from_100_hz() {
        let (_h, mut shell) = home_shell("settings");
        assert!(shell.kit.modal_beat_ms() > 0, "60 Hz until the compositor says: the layer beat, {} ms", shell.kit.modal_beat_ms());
        shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 120}) }, 1_500);
        assert_eq!(shell.kit.modal_beat_ms(), 0, "120 Hz: no fade");
        shell.channel(&Event::Topic { topic: "refresh".into(), data: json!({"hz": 60}) }, 1_600);
        assert!(shell.kit.modal_beat_ms() > 0, "back at 60 Hz: the fade is back");
        shell.channel(&Event::State([("refresh".to_string(), json!({"hz": 120}))].into_iter().collect()), 1_700);
        assert_eq!(shell.kit.modal_beat_ms(), 0, "a session that starts at 120 Hz knows it from the state");
    }

    #[test]
    fn helper_events_that_come_together_are_drawn_and_reported_once_and_leave_the_screen_as_one_by_one_would() {
        let events: Vec<Event> = (1..=4).map(|i| Event::Topic { topic: "weather".into(), data: json!({"doc": {"fetchedAt": i, "display": {"place": "Atlanta", "temp": format!("{}\u{b0}", 60 + i), "cond": "Clear", "days": []}}}) }).collect();
        let (_a, mut together) = home_shell("home");
        let (_b, mut apart) = home_shell("home");
        let (built_together, built_apart) = (together.reports_built, apart.reports_built);
        together.helper_all(&events, 2_000);
        for e in &events {
            apart.helper(e, 2_000);
        }
        assert_eq!(together.reports_built, built_together + 1, "four events, one report");
        assert_eq!(apart.reports_built, built_apart + 4);
        assert_eq!(together.report(), apart.report(), "the same screen in the end");
    }

    #[test]
    fn a_pointer_that_only_moves_does_not_rebuild_the_report_and_never_leaves_it_stale() {
        let (_h, mut shell) = home_shell("home");
        let mut at = 2_000;
        // the pointer shown over nothing, then jittering there
        shell.key(&KeyEvent::Pointer { x: 5.0, y: 5.0, buttons: 0, visible: true }, at);
        let built = shell.reports_built;
        for i in 0..50 {
            at += 16;
            shell.key(&KeyEvent::Pointer { x: 5.0 + (i % 7) as f32, y: 5.0, buttons: 0, visible: true }, at);
        }
        assert_eq!(shell.reports_built, built, "motion that changed nothing built no report");
        // a button or the pointer's hiding is more than motion
        shell.key(&KeyEvent::Pointer { x: 5.0, y: 5.0, buttons: 1, visible: true }, at + 16);
        shell.key(&KeyEvent::Pointer { x: 5.0, y: 5.0, buttons: 0, visible: true }, at + 32);
        assert_eq!(shell.reports_built, built + 2);
        // across the whole screen: whatever the motion did, the report the compositor holds is the one the shell would send now
        let mut rebuilt = 0;
        for i in 0..400u32 {
            at += 16;
            let before = shell.reports_built;
            shell.key(&KeyEvent::Pointer { x: ((i * 97) % 1920) as f32, y: ((i * 53) % 1080) as f32, buttons: 0, visible: true }, at);
            rebuilt += shell.reports_built - before;
            assert_eq!(shell.reported, Some(shell.report()), "stale after sample {i}");
        }
        assert!(rebuilt < 400, "some of the sweep's motion changed nothing: {rebuilt} of 400 built a report");
    }

    #[test]
    fn a_theme_file_that_is_as_it_was_is_not_read_again_and_one_that_changed_applies() {
        use std::fs::File;
        let (_h, mut shell) = home_shell("home");
        let dir = std::env::temp_dir().join(format!("lumatv-poll-skip-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        let path = dir.join("theme/theme.json");
        let base = SystemTime::now();
        // a negative age is a time ahead of the clock: not yet old enough to tell a rewrite, and a slow test cannot age it
        let write = |style: &str, age: i64| {
            std::fs::write(&path, format!(r#"{{"clock":{{"style":"{style}"}}}}"#)).unwrap();
            let time = if age >= 0 { base - Duration::from_secs(age as u64) } else { base + Duration::from_secs(age.unsigned_abs()) };
            File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
        };
        write("aaaaaa", 3600);
        shell.dirs(&testkit::module_dir(), &dir);
        let mut at = 100_000;
        shell.tick(at);
        assert_eq!(shell.home.theme["clock"]["style"], "aaaaaa", "the first poll reads it");
        // the same size and time: not read, so not seen
        write("bbbbbb", 3600);
        at += 2_100;
        shell.tick(at);
        assert_eq!(shell.home.theme["clock"]["style"], "aaaaaa");
        // a new time: read and applied
        write("bbbbbb", 3000);
        at += 2_100;
        shell.tick(at);
        assert_eq!(shell.home.theme["clock"]["style"], "bbbbbb");
        // written just now, too soon for the time to tell: read at every poll until it has aged
        write("cccccc", -60);
        at += 2_100;
        shell.tick(at);
        assert_eq!(shell.home.theme["clock"]["style"], "cccccc");
        write("dddddd", -60);
        at += 2_100;
        shell.tick(at);
        assert_eq!(shell.home.theme["clock"]["style"], "dddddd");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nitin's cards, every kind the compositor starts the shell with, motion off: once what is due has been done, nothing is due until the
    /// clock's next second, on Home and behind an app. A timer that is always due (the closed guide's fetch was, at 0) turns the loop at full
    /// speed and each turn commits Home's poses again, which the compositor draws as a frame at every refresh.
    #[test]
    fn a_shell_with_every_kind_and_nitins_cards_asks_for_no_tick_at_rest_on_home_and_behind_an_app() {
        let (_h, kit) = testkit::kit(1.0);
        let kinds = "home,osd,settings,toast,cameras,quick,picker,drawer,menu,confirm,switcher,entry,keyboard,alert,saver,capture,cards,popup,layout,ai,banner,displaylab,guide";
        let theme = json!({"wallpaperMotion": {"enabled": false}, "weather": {"animated": false}});
        let layout = json!({"grid": {"rows": 16}, "widgets": [
            {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 7, "rows": 4},
            {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 7, "rows": 5, "title": "At home", "hidden": true},
            {"id": "weather", "type": "weather", "col": 7, "row": 0, "span": 5, "rows": 9},
            {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110},
            {"id": "onhome", "type": "onhome", "col": 0, "row": 4, "span": 7, "rows": 5, "title": "Smart home"}]});
        let mut shell = Shell::new(kit, Kinds::parse(kinds), 1000, &testkit::module_dir().join("icons/audio"), &theme, &layout).expect("a shell");
        shell.start();
        let mut now = crate::host::now_ms();
        for front in [json!({"app": "com.webos.app.home", "home": true, "covered": false}), json!({"app": "com.webos.app.livetv", "home": false, "covered": true})] {
            shell.channel(&Event::Topic { topic: "foreground".into(), data: front.clone() }, now);
            let mut ticks = 0;
            while shell.due_at().is_some_and(|due| due <= now) {
                shell.tick(now);
                ticks += 1;
                assert!(ticks < 20, "{front}: still due at {now} after {ticks} ticks: {:?}", shell.due_at());
                now += 8;
            }
            assert!(shell.due_at().is_none_or(|due| due > now), "{front}: {:?} at {now}", shell.due_at());
        }
    }
}
