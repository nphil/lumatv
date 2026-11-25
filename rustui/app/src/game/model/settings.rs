//! Settings: five places on a rail, each a list of rows, and the pages a row opens (`Pane`): an account or a PC, adding one, the
//! themes, the controllers' lab, the GameSir and Leadjoy pages and diagnostics.
use super::nav::{Hints, Slice};
use super::*;

/// A page a Settings row opens in place of its place's list; `View::pane` holds the one that is open, and Back returns to the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// An integration, or one PC of it: `View::selected_engine` and `View::selected_target`.
    Service,
    Add,
    Theme,
    Lab,
    GameSir,
    Leadjoy,
    Diagnostics,
    /// The recorder's settings, shared by every integration that keeps one.
    Recorder,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Place {
    Accounts,
    Picture,
    Controllers,
    #[default]
    Appearance,
    System,
}
impl Place {
    pub const ALL: [Self; 5] = [Self::Accounts, Self::Picture, Self::Controllers, Self::Appearance, Self::System];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        ["Accounts and PCs", "Picture and stream", "Controllers", "Appearance", "System"][self.index()]
    }
}
/// What a row leads with. The screen picks the icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Cloud,
    Pc,
    Tv,
    Account,
    Add,
    Clock,
    Games,
    News,
    Mature,
    SignOut,
    Qr,
    Picture,
    Motion,
    Pad,
    Palette,
    Light,
    Text,
    Gauge,
    Info,
    Bug,
    Setting,
}
/// The colour of a status word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Positive,
    Info,
    Warning,
    Danger,
    Quiet,
}
/// What a row shows at its right.
#[derive(Clone, Debug, PartialEq)]
pub enum Shows {
    /// A chevron: OK opens something.
    Opens,
    Value(String),
    /// A status word and its mark.
    State(&'static str, Tone),
    Switch(bool),
    /// Left and Right step through choices.
    Stepper(String),
    Nothing,
}
/// What OK, Left and Right on a row do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    None,
    Pane(Pane),
    Service(EngineId, Option<TargetId>),
    ControllerSettings,
    Pref(Pref),
    /// A diagnostics row, by its place in the pane: on, level, server, topic, token, clear the token, test.
    Diag(usize),
    Link(EngineId, Option<TargetId>),
    /// Links an account with a key typed on the TV (Steam's Web API key) rather than a sign-in.
    LinkKey(EngineId),
    AddHost(EngineId),
    SignOut,
    /// A field of the open integration's schema, by its place in the schema.
    Field(usize),
    /// A field of the Game recorder page, by its place in the recorder's section.
    Recorder(usize),
    Credit(&'static str),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pref {
    Resolution,
    FrameRate,
    Hdr,
    Bitrate,
    Codec,
    Pacing,
    Preferred,
    Stats,
    Light,
    Covers,
    Text,
    Motion,
    Labels,
    GamePages,
    MaturePages,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub act: Act,
    pub glyph: Glyph,
    pub name: String,
    pub sub: Option<String>,
    pub shows: Shows,
    /// Lets something go: its mark reads in the danger colour.
    pub danger: bool,
}
impl Item {
    pub(super) fn new(act: Act, glyph: Glyph, name: impl Into<String>, shows: Shows) -> Self {
        Self { act, glyph, name: name.into(), sub: None, shows, danger: false }
    }
    pub(super) fn sub(mut self, sub: impl Into<String>) -> Self {
        self.sub = Some(sub.into());
        self
    }
}
/// One line of a Settings list. `Focus::Setting(i)` is the `i`th entry; groups and notes take no focus.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Group(String),
    Note(String),
    Row(Item),
}
/// What the open place or pane shows: its head, a line under it and its list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub crumb: Option<&'static str>,
    pub title: String,
    pub chip: Option<(&'static str, Tone)>,
    pub lede: Option<String>,
    pub entries: Vec<Entry>,
}
impl Sheet {
    fn titled(title: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self { title: title.into(), entries, ..Self::default() }
    }
    /// The entries that take focus.
    pub fn rows(&self) -> Vec<usize> {
        self.entries.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Row(_))).map(|(i, _)| i).collect()
    }
    pub fn item(&self, i: usize) -> Option<&Item> {
        match self.entries.get(i) {
            Some(Entry::Row(item)) => Some(item),
            _ => None,
        }
    }
}
/// The log levels, as the diagnostics pane names them.
const LEVELS: [&str; 5] = ["Error", "Warn", "Info", "Debug", "Trace"];
/// How the shell reads the pads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadMode {
    // Discriminants are also the shell's atomic input mode.
    /// As the menu's keys.
    Menu,
    /// For the lab's live picture, and as the menu's keys without the sticks, which are under test.
    Lab,
    /// For the lab alone: a test has every button until B is held for [`LEAVE_HOLD_NS`] or the remote's Back is pressed.
    Captured,
}
/// A lab test that has every button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    Buttons,
    /// The screen lights on the frame after a press, for a slow-motion video to time against the thumb.
    Flash,
}
/// The stream round trip the lab asked the running engine for.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Trip {
    #[default]
    Idle,
    Waiting(EngineId),
    Answer(EngineId, ipc::RoundTrip),
    /// The engine does not time it: no `round-trip` feature, or an older engine that cannot read the request.
    Unsupported(EngineId),
}
/// Holding B this long leaves a test that has every button.
pub const LEAVE_HOLD_NS: u64 = 1_000_000_000;
/// The lab's figures change at most this often; the dots and bars follow every report.
pub const LAB_REFRESH_NS: u64 = 100_000_000;
#[derive(Clone, Debug, Default)]
pub struct LabPad {
    pub name: String,
    /// The latest report, before any dead zone.
    pub report: RawReport,
    pub timing: Timing,
    pub summary: Summary,
    /// Left and right stick.
    pub drift: [Drift; 2],
    /// The kernel's stamp of a B press still held.
    b_down: Option<u64>,
}
#[derive(Clone, Debug, Default)]
pub struct Lab {
    pub pads: [Option<LabPad>; MAX_PADS],
    /// The pad that reported last, which the lab draws.
    pub active: Option<usize>,
    pub capture: Option<Capture>,
    /// The kernel's stamp of the press that lit the flash, while any button is held.
    pub flash: Option<u64>,
    /// The number of the frame the shell draws next, kept while the flash test runs.
    pub frame: u64,
    pub trip: Trip,
    refreshed_ns: u64,
}
impl Lab {
    pub fn connected(&self) -> usize {
        self.pads.iter().flatten().count()
    }
    /// The pad the lab draws: the one that reported last, else the first connected.
    pub fn pad(&self) -> Option<&LabPad> {
        self.active.and_then(|i| self.pads[i].as_ref()).or_else(|| self.pads.iter().flatten().next())
    }
}
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.settings_grid(layer)
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.settings_activate(f, out);
    }
    fn adjust(m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool {
        m.settings_adjust(f, forward, out)
    }
    fn anchor(m: &Model) -> Option<Focus> {
        m.view.pane.is_none().then(|| Focus::Rail(m.view.place.index()))
    }
    fn hints(m: &Model) -> Hints {
        m.settings_hints()
    }
}
/// What a schema field's row shows at its right, from its current value.
pub(super) fn field_shows(kind: &FieldKind, current: Option<Value>) -> Shows {
    match (kind, current) {
        (FieldKind::Toggle { .. }, Some(Value::Bool(on))) => Shows::Switch(on),
        (FieldKind::Choice { options, .. }, Some(Value::Text(v))) => Shows::Stepper(options.iter().find(|o| o.id == v).map_or(v.clone(), |o| o.label.clone())),
        (FieldKind::Number { unit, .. }, Some(Value::Int(v))) => Shows::Stepper(format!("{v}{}", unit.as_deref().unwrap_or(""))),
        (FieldKind::Text { .. }, Some(Value::Text(v))) if !v.is_empty() => Shows::Value(v),
        (FieldKind::Text { .. }, _) => Shows::Value("Add".into()),
        _ => Shows::Opens,
    }
}
/// The value a field takes when Left or Right steps it: a switch flips, a choice and a number move one place and stop at the ends.
pub(super) fn step_value(kind: &FieldKind, current: Option<Value>, right: bool) -> Option<Value> {
    match (kind, current) {
        (FieldKind::Toggle { .. }, Some(Value::Bool(v))) => Some(Value::Bool(!v)),
        (FieldKind::Choice { options, .. }, Some(Value::Text(v))) if !options.is_empty() => {
            let i = options.iter().position(|o| o.id == v).unwrap_or(0);
            let j = if right { (i + 1).min(options.len() - 1) } else { i.saturating_sub(1) };
            Some(Value::Text(options[j].id.clone()))
        }
        (FieldKind::Number { min, max, step, .. }, Some(Value::Int(v))) => Some(Value::Int(if right { v.saturating_add(*step).min(*max) } else { v.saturating_sub(*step).max(*min) })),
        _ => None,
    }
}
impl Model {
    /// What the Game recorder row says under its name.
    fn recorder_word(&self) -> &'static str {
        if lumagame_core::recorder::Settings::from_values(&self.recorder_values()).runs() { "On" } else { "Off" }
    }
    /// Shows Settings at `place`, focused on its rail.
    pub fn open_settings(&mut self, place: Place) {
        self.view.place = place;
        self.show_page(Page::Settings, Focus::Rail(place.index()));
    }
    /// The diagnostics pane is open.
    pub fn in_diagnostics(&self) -> bool {
        self.view.page == Page::Settings && self.view.pane == Some(Pane::Diagnostics)
    }
    /// The open place or pane: its head and rows.
    pub fn settings_sheet(&self) -> Sheet {
        let place = self.view.place;
        let mut sheet = match self.view.pane {
            None => match place {
                Place::Accounts => self.accounts_sheet(),
                Place::Picture => Sheet::titled(place.name(), self.picture_entries()),
                Place::Controllers => Sheet::titled(place.name(), self.controller_entries()),
                Place::Appearance => Sheet::titled(place.name(), self.appearance_entries()),
                Place::System => Sheet::titled(place.name(), self.system_entries()),
            },
            Some(Pane::Service) => self.service_sheet(),
            Some(Pane::Add) => Sheet::titled("Add an account or PC", self.add_entries()),
            Some(Pane::Theme) => Sheet::titled("Theme", Vec::new()),
            Some(Pane::Lab) => Sheet::titled("Controller lab", Vec::new()),
            Some(Pane::GameSir) => Sheet::titled("GameSir Cyclone 2", Vec::new()),
            Some(Pane::Leadjoy) => Sheet::titled("Leadjoy Xeno", Vec::new()),
            Some(Pane::Diagnostics) => Sheet::titled("Diagnostics", self.diagnostics_entries()),
            Some(Pane::Recorder) => Sheet::titled("Game recorder", self.recorder_entries()),
        };
        if self.view.pane.is_some() {
            sheet.crumb = Some(place.name());
        }
        sheet
    }
    /// Each place's state, under its name on the rail.
    pub fn rail_states(&self) -> [String; 5] {
        let targets: Vec<&Target> = self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).flat_map(|e| e.targets.iter()).collect();
        let offline = targets.iter().filter(|t| t.reach == ipc::Reach::Offline).count();
        let accounts = if offline > 0 {
            format!("{offline} offline")
        } else if self.attention().is_some() {
            "Needs attention".into()
        } else if targets.is_empty() {
            "Nothing linked yet".into()
        } else {
            "All ready".into()
        };
        [
            accounts,
            format!("{}, {} fps", self.resolution_name(), self.prefs.fps),
            self.prefs.button_labels.name().into(),
            self.theme_name().into(),
            format!("Version {}", env!("CARGO_PKG_VERSION")),
        ]
    }
    fn resolution_name(&self) -> String {
        if self.prefs.height == 2160 { "4K".into() } else { format!("{}p", self.prefs.height) }
    }
    fn theme_name(&self) -> &'static str {
        lumatv_tokens::themes::game_options().iter().find(|f| f.id == self.prefs.family).map_or("Rosé Pine", |f| f.name)
    }
    /// The theme pane's columns.
    pub fn theme_columns(&self) -> usize {
        if self.prefs.text_percent > 100 { 3 } else { 4 }
    }
    fn picture_entries(&self) -> Vec<Entry> {
        let p = &self.prefs;
        let codec = match p.codec {
            Some(Codec::H264) => "H.264",
            Some(Codec::H265) => "HEVC",
            Some(Codec::Av1) => "AV1",
            _ => "Automatic",
        };
        let mut codec = Item::new(Act::Pref(Pref::Codec), Glyph::Setting, "Codec", Shows::Stepper(codec.into()));
        if p.fps > 60 {
            codec = codec.sub("AV1 is available at 60 fps");
        }
        let mut bitrate = Item::new(Act::Pref(Pref::Bitrate), Glyph::Gauge, "Bitrate", Shows::Stepper(format!("{} Mb/s", p.bitrate_mbps)));
        // The stepper keeps the viewer's choice; what the TV's port lets through is said under it.
        if let Some(note) = self.uplink.as_ref().and_then(|u| u.note(p.bitrate_mbps)) {
            bitrate = bitrate.sub(note);
        }
        let preferred = self.state().profile().user.preferred_engine.as_ref().map_or("Ask each time", |id| self.engine_name(id)).to_owned();
        vec![
            Entry::Row(Item::new(Act::Pref(Pref::Resolution), Glyph::Picture, "Resolution", Shows::Stepper(self.resolution_name()))),
            Entry::Row(Item::new(Act::Pref(Pref::FrameRate), Glyph::Motion, "Frame rate", Shows::Stepper(format!("{} fps", p.fps)))),
            Entry::Row(Item::new(Act::Pref(Pref::Hdr), Glyph::Light, "HDR", Shows::Switch(p.hdr))),
            Entry::Row(bitrate),
            Entry::Row(codec),
            Entry::Row(Item::new(Act::Pref(Pref::Pacing), Glyph::Clock, "Frame pacing", Shows::Stepper(if p.smooth_pacing { "Smooth" } else { "Lowest latency" }.into()))),
            Entry::Group("Playing".into()),
            Entry::Row(Item::new(Act::Pref(Pref::Preferred), Glyph::Games, "Preferred way to play", Shows::Stepper(preferred))),
            Entry::Row(Item::new(Act::Pref(Pref::Stats), Glyph::Gauge, "Stats while playing", Shows::Stepper(["Off", "Compact", "Full"][p.stats.mode.index()].into()))),
            Entry::Group("Recording".into()),
            Entry::Row(Item::new(Act::Pane(Pane::Recorder), Glyph::Picture, "Game recorder", Shows::Opens).sub(self.recorder_word())),
        ]
    }
    fn controller_entries(&self) -> Vec<Entry> {
        vec![
            Entry::Row(Item::new(Act::Pane(Pane::Lab), Glyph::Pad, "Controller lab", Shows::Opens).sub("Buttons, sticks, rumble and timing")),
            Entry::Row(Item::new(Act::ControllerSettings, Glyph::Setting, "TV controller settings", Shows::Opens).sub("Pair a Bluetooth controller")),
            Entry::Row(Item::new(Act::Pane(Pane::GameSir), Glyph::Pad, "GameSir Cyclone 2", Shows::Opens).sub("Settings stored on the controller")),
            Entry::Row(Item::new(Act::Pane(Pane::Leadjoy), Glyph::Pad, "Leadjoy Xeno", Shows::Opens).sub("Controller settings and logo lighting")),
            Entry::Row(
                Item::new(Act::Pref(Pref::Labels), Glyph::Text, "Button labels", Shows::Stepper(self.prefs.button_labels.name().into()))
                    .sub("The markings hints show on face buttons"),
            ),
        ]
    }
    fn appearance_entries(&self) -> Vec<Entry> {
        let p = &self.prefs;
        vec![
            Entry::Row(Item::new(Act::Pane(Pane::Theme), Glyph::Palette, "Theme", Shows::Opens).sub(self.theme_name())),
            Entry::Row(Item::new(Act::Pref(Pref::Light), Glyph::Light, "Light", Shows::Switch(p.light))),
            Entry::Row(Item::new(Act::Pref(Pref::Covers), Glyph::Games, "Cover size", Shows::Stepper(["Compact", "Standard", "Large"][p.covers.index()].into()))),
            Entry::Row(Item::new(Act::Pref(Pref::Text), Glyph::Text, "Text size", Shows::Stepper(format!("{}%", p.text_percent)))),
            Entry::Row(Item::new(Act::Pref(Pref::Motion), Glyph::Motion, "Reduce motion", Shows::Switch(p.reduce_motion)).sub("Focus changes at once, with no lift or fades")),
        ]
    }
    fn system_entries(&self) -> Vec<Entry> {
        let credit = |name: &str, text: &str, url| Entry::Row(Item::new(Act::Credit(url), Glyph::Info, name, Shows::Opens).sub(text));
        vec![
            Entry::Row(Item::new(Act::Pane(Pane::Diagnostics), Glyph::Bug, "Diagnostics", Shows::Opens).sub(self.diagnostics_word())),
            Entry::Group("About".into()),
            Entry::Row(Item::new(Act::None, Glyph::Info, "Version", Shows::Value(env!("CARGO_PKG_VERSION").into()))),
            credit("Steam", lumagame_meta::STEAM_CREDIT, "https://store.steampowered.com"),
            credit("SteamGridDB", lumagame_meta::SGDB_CREDIT, "https://steamgriddb.com"),
            credit("IGDB", lumagame_meta::IGDB_CREDIT, "https://igdb.com"),
        ]
    }
    fn diagnostics_word(&self) -> &'static str {
        use crate::game::diagnostics::Status;
        match self.diagnostics_status {
            Status::Off => "Off",
            Status::NotConfigured => "Add an HTTPS server and topic",
            Status::Waiting => "Ready",
            Status::Sending => "Sending",
            Status::Sent => "Last send succeeded",
            Status::Error => "Send failed",
        }
    }
    fn diagnostics_entries(&self) -> Vec<Entry> {
        let d = &self.diagnostics;
        // Short words: a long one would be cut off at large text sizes.
        let note = ["Failures only", "Plus degraded streams", "Plus session and TV state", "Plus pacing and decode stats", "Plus fine timing detail"][d.level.index()];
        let text = |v: &str| Shows::Value(if v.is_empty() { "Add".into() } else { v.into() });
        vec![
            Entry::Row(Item::new(Act::Diag(0), Glyph::Bug, "Remote diagnostics", Shows::Switch(d.enabled)).sub(format!("{} · Off stops sending; delivered messages remain", self.diagnostics_word()))),
            Entry::Row(Item::new(Act::Diag(1), Glyph::Text, "Log level", Shows::Stepper(LEVELS[d.level.index()].into())).sub(note)),
            Entry::Row(Item::new(Act::Diag(2), Glyph::Setting, "Server URL", text(&d.server))),
            Entry::Row(Item::new(Act::Diag(3), Glyph::Setting, "Topic", text(&d.topic))),
            Entry::Row(Item::new(Act::Diag(4), Glyph::Setting, "Write token", Shows::Value(if d.token.is_empty() { "Add" } else { "Set" }.into()))),
            Entry::Row(Item::new(Act::Diag(5), Glyph::SignOut, "Clear write token", Shows::Nothing)),
            Entry::Row(Item::new(Act::Diag(6), Glyph::Info, "Test diagnostics", Shows::Value("Send".into()))),
        ]
    }
    /// The Game recorder page: every field of the recorder's section, with the values every integration that keeps one shares.
    fn recorder_entries(&self) -> Vec<Entry> {
        let values = self.recorder_values();
        lumagame_core::recorder::section()
            .fields
            .into_iter()
            .enumerate()
            .map(|(i, field)| {
                let mut item = Item::new(Act::Recorder(i), Glyph::Setting, field.label.clone(), field_shows(&field.kind, values.get(&field.key).cloned()));
                item.sub = field.help;
                Entry::Row(item)
            })
            .collect()
    }
    fn settings_grid(&self, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        match (layer, self.view.pane) {
            (Some(Layer::Delete), _) => rows.push(vec![Focus::SheetAction(0), Focus::SheetAction(1)]),
            (Some(_), _) => {}
            (None, None) => {
                let entries = self.settings_sheet().rows();
                for r in 0..entries.len().max(Place::ALL.len()) {
                    let mut row = Vec::new();
                    if r < Place::ALL.len() {
                        row.push(Focus::Rail(r));
                    }
                    if let Some(&i) = entries.get(r) {
                        row.push(Focus::Setting(i));
                    }
                    rows.push(row);
                }
            }
            (None, Some(Pane::Theme)) => {
                let (count, cols) = (lumatv_tokens::themes::game_options().len(), self.theme_columns());
                for i in (0..count).step_by(cols) {
                    rows.push((i..(i + cols).min(count)).map(Focus::Theme).collect());
                }
            }
            (None, Some(Pane::Lab)) => {
                if !self.session_active() {
                    if self.prefs.text_percent > 100 { rows.extend([vec![Focus::Lab(0), Focus::Lab(1)], vec![Focus::Lab(2), Focus::Lab(3)]]); } else { rows.push((0..4).map(Focus::Lab).collect()); }
                    if self.lab.pad().is_some() {
                        rows.extend((5..8).map(|i| vec![Focus::Lab(i)]));
                    }
                } else if self.round_trip_engine().is_some() && !matches!(self.lab.trip, Trip::Unsupported(_)) {
                    rows.push(vec![Focus::Lab(4)]);
                    if matches!(self.lab.trip, Trip::Answer(..)) {
                        rows.extend((8..13).map(|i| vec![Focus::Lab(i)]));
                    }
                }
                rows.extend((0..3).map(|i| vec![Focus::Setting(i)]));
            }
            (None, Some(Pane::GameSir)) => {
                let count = if matches!(self.gamesir.state, GameSirState::Ready(_)) { 8 } else { 1 };
                rows.extend((0..count).map(|i| vec![Focus::Setting(i)]));
            }
            (None, Some(Pane::Leadjoy)) => {
                rows.push(vec![Focus::Setting(0)]);
                if let LeadjoyState::Ready(profile) = &self.leadjoy.state {
                    rows.extend(profile.schema.fields().enumerate().filter(|(_, field)| !matches!(field.kind, FieldKind::Note))
                        .map(|(i, _)| vec![Focus::Setting(i + 1)]));
                }
            }
            (None, Some(_)) => rows.extend(self.settings_sheet().rows().into_iter().map(|i| vec![Focus::Setting(i)])),
        }
        rows
    }
    /// The row Right from the rail reaches: the one the place last had, else its first.
    fn settings_entry_row(&self, entries: &[usize]) -> Option<usize> {
        let kept = self.view.settings_rows[self.view.place.index()];
        if entries.contains(&kept) { Some(kept) } else { entries.first().copied() }
    }
    /// The rail and the place's rows link to each other explicitly: Up and Down on the rail step the places, Right enters the place at
    /// the row it had, and every row's Left returns to its place.
    pub(super) fn settings_links(&mut self) {
        let entries = self.settings_sheet().rows();
        let into = self.settings_entry_row(&entries);
        let place = self.view.place.index();
        for n in &mut self.navigation {
            match n.id {
                Focus::Rail(i) => {
                    n.left = None;
                    n.right = into.map(Focus::Setting);
                    n.up = i.checked_sub(1).map(Focus::Rail);
                    n.down = (i + 1 < Place::ALL.len()).then_some(Focus::Rail(i + 1));
                }
                Focus::Setting(i) => {
                    let at = entries.iter().position(|&j| j == i);
                    n.left = Some(Focus::Rail(place));
                    n.right = None;
                    n.up = at.and_then(|a| a.checked_sub(1)).map(|a| Focus::Setting(entries[a]));
                    n.down = at.and_then(|a| entries.get(a + 1)).map(|&j| Focus::Setting(j));
                }
                _ => {}
            }
        }
    }
    fn settings_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Rail(i) => {
                self.view.place = Place::ALL[i];
                if let Some(row) = self.settings_entry_row(&self.settings_sheet().rows()) {
                    self.set_focus(Focus::Setting(row));
                }
            }
            Focus::Setting(_) if matches!(self.view.pane, Some(Pane::Lab | Pane::GameSir | Pane::Leadjoy)) => {
                self.adjust(true, out);
            }
            Focus::Setting(i) => {
                if let Some(item) = self.settings_sheet().item(i).cloned() {
                    self.settings_act(item.act, out);
                }
            }
            Focus::Lab(i) => self.lab_action(i, out),
            Focus::Theme(i) => {
                let families = lumatv_tokens::themes::game_catalog();
                self.prefs.family = if i == 0 { "match-lumatv".into() } else { families.get(i - 1).map_or("rose-pine", |f| f.id).into() };
                self.save_prefs(out);
                self.go_back(out);
            }
            Focus::SheetAction(i) if self.view.top() == Some(Layer::Delete) => {
                if i == 1 { self.delete_leadjoy(out); }
                self.pop_layer();
            }
            _ => {}
        }
    }
    fn settings_act(&mut self, act: Act, out: &mut Vec<Command>) {
        match act {
            Act::None => {}
            Act::Pane(pane) => self.open_pane(pane, out),
            Act::Service(engine, target) => self.open_service(engine, target, out),
            Act::ControllerSettings => out.push(Command::OpenControllerSettings),
            Act::Pref(pref) => {
                self.set_pref(pref, None, out);
            }
            Act::Diag(row) => self.diagnostics_action(row, out),
            Act::Link(engine, target) => self.begin_link(engine, target, out),
            Act::LinkKey(engine) => self.begin_key_link(engine, out),
            Act::AddHost(engine) => {
                self.view.selected_engine = Some(engine);
                self.view.address.clear();
                self.push_layer(Layer::AddHost, Focus::Keyboard(0));
            }
            Act::SignOut => self.push_layer(Layer::SignOut, Focus::SheetAction(0)),
            Act::Field(i) => self.change_engine_field(i, true, out),
            Act::Recorder(i) => {
                self.change_recorder_field(i, true, out);
            }
            Act::Credit(url) => out.push(Command::PlayMedia { url: url.into() }),
        }
    }
    fn open_pane(&mut self, pane: Pane, out: &mut Vec<Command>) {
        match pane {
            Pane::Lab => {
                self.lab = Lab::default();
                let focus = if !self.session_active() {
                    Focus::Lab(0)
                } else if self.round_trip_engine().is_some() {
                    Focus::Lab(4)
                } else {
                    Focus::Setting(0)
                };
                self.show_pane(pane, focus);
            }
            Pane::GameSir => {
                self.show_pane(pane, Focus::Setting(0));
                self.read_gamesir(out);
            }
            Pane::Leadjoy => {
                self.show_pane(pane, Focus::Setting(0));
                self.read_leadjoy(out);
            }
            Pane::Theme => {
                let i = lumatv_tokens::themes::game_options().iter().position(|o| o.id == self.prefs.family).unwrap_or(0);
                self.show_pane(pane, Focus::Theme(i));
            }
            Pane::Service | Pane::Add | Pane::Diagnostics | Pane::Recorder => self.show_pane(pane, Focus::Setting(0)),
        }
    }
    fn diagnostics_action(&mut self, row: usize, out: &mut Vec<Command>) {
        match row {
            0 => {
                self.diagnostics.enabled = !self.diagnostics.enabled;
                out.push(Command::Diagnostics(self.diagnostics.clone()));
                self.stats_request(out);
            }
            1 => {
                self.diagnostics.level = DiagnosticsLevel::ALL[(self.diagnostics.level.index() + 1) % DiagnosticsLevel::ALL.len()];
                out.push(Command::Diagnostics(self.diagnostics.clone()));
            }
            2..=4 => {
                self.view.edit_field = Some(row);
                self.view.address = match row { 2 => self.diagnostics.server.clone(), 3 => self.diagnostics.topic.clone(), _ => String::new() };
                self.push_layer(Layer::EditText, Focus::Keyboard(0));
            }
            5 => {
                self.diagnostics.token.clear();
                out.push(Command::Diagnostics(self.diagnostics.clone()));
            }
            6 => out.push(Command::DiagnosticTest),
            _ => {}
        }
    }
    /// Left and Right on a row: a value steps, and true says it changed. Left on a pane's row that holds no value returns to the row
    /// that opened the pane.
    fn settings_adjust(&mut self, f: Focus, right: bool, out: &mut Vec<Command>) -> bool {
        let Focus::Setting(row) = f else {
            return false;
        };
        match self.view.pane {
            Some(Pane::GameSir) => return self.adjust_gamesir(row, right, out),
            Some(Pane::Leadjoy) => return self.adjust_leadjoy(row, right, out),
            Some(Pane::Lab) => {
                let nudge = |v: u8, max: u8| if right { (v + 1).min(max) } else { v.saturating_sub(1) };
                let curves = [Curve::Linear, Curve::Precise, Curve::Quick];
                match row {
                    0 => self.prefs.deadzone = nudge(self.prefs.deadzone, DEADZONE_MAX),
                    1 => self.prefs.anti_deadzone = nudge(self.prefs.anti_deadzone, ANTI_DEADZONE_MAX),
                    2 => {
                        let at = curves.iter().position(|c| *c == self.prefs.stick_curve).unwrap_or(0);
                        self.prefs.stick_curve = curves[if right { (at + 1).min(curves.len() - 1) } else { at.saturating_sub(1) }];
                    }
                    _ => return false,
                }
                self.save_prefs(out);
                self.push_common(out);
                return true;
            }
            _ => {}
        }
        let Some(item) = self.settings_sheet().item(row).cloned() else {
            return false;
        };
        if matches!(item.shows, Shows::Switch(_) | Shows::Stepper(_)) {
            return match item.act {
                Act::Pref(pref) => self.set_pref(pref, Some(right), out),
                Act::Diag(row) => self.diagnostics_step(row, right, out),
                Act::Field(i) => {
                    self.change_engine_field(i, right, out);
                    true
                }
                Act::Recorder(i) => self.change_recorder_field(i, right, out),
                _ => false,
            };
        }
        if self.view.pane.is_some() && !right {
            self.go_back(out);
            return true;
        }
        false
    }
    /// A field of the Game recorder page: a switch or a choice steps, a text opens the keyboard. True when it changed or opened.
    fn change_recorder_field(&mut self, index: usize, right: bool, out: &mut Vec<Command>) -> bool {
        let Some(field) = lumagame_core::recorder::section().fields.into_iter().nth(index) else {
            return false;
        };
        let current = self.recorder_values().get(&field.key).cloned();
        if let FieldKind::Text { .. } = field.kind {
            self.view.address = if let Some(Value::Text(value)) = current { value } else { String::new() };
            self.view.edit_field = Some(index);
            self.push_layer(Layer::EditText, Focus::Keyboard(0));
            return true;
        }
        let Some(value) = step_value(&field.kind, current.clone(), right).filter(|v| Some(v) != current.as_ref()) else {
            return false;
        };
        self.prefs.recorder.insert(field.key, value);
        self.save_prefs(out);
        self.push_common(out);
        true
    }
    /// The Game recorder page's text field, from the keyboard.
    pub(super) fn save_recorder_text(&mut self, out: &mut Vec<Command>) {
        let Some(field) = self.view.edit_field.and_then(|i| lumagame_core::recorder::section().fields.into_iter().nth(i)) else {
            return;
        };
        let value = Value::Text(self.view.address.trim().to_owned());
        let schema = SettingsSchema { sections: vec![lumagame_core::recorder::section()] };
        if schema.check(&field.key, &value).is_err() {
            self.view.notice = Some(match field.kind {
                FieldKind::Text { max_chars, .. } => format!("Use at most {max_chars} characters."),
                _ => "This field changed. Open it again.".into(),
            });
            return;
        }
        self.prefs.recorder.insert(field.key, value);
        self.save_prefs(out);
        self.push_common(out);
        self.pop_layer();
    }
    fn diagnostics_step(&mut self, row: usize, right: bool, out: &mut Vec<Command>) -> bool {
        let before = (self.diagnostics.enabled, self.diagnostics.level);
        match row {
            0 => self.diagnostics.enabled = right,
            1 => {
                let i = self.diagnostics.level.index();
                self.diagnostics.level = DiagnosticsLevel::ALL[if right { (i + 1).min(DiagnosticsLevel::ALL.len() - 1) } else { i.saturating_sub(1) }];
            }
            _ => return false,
        }
        if before == (self.diagnostics.enabled, self.diagnostics.level) {
            return false;
        }
        out.push(Command::Diagnostics(self.diagnostics.clone()));
        self.stats_request(out);
        true
    }
    /// Steps a preference: `dir` Some(right) stops at either end, None (OK) goes round. True when it changed.
    fn set_pref(&mut self, pref: Pref, dir: Option<bool>, out: &mut Vec<Command>) -> bool {
        let step = |i: usize, len: usize| match dir {
            None => (i + 1) % len,
            Some(true) => (i + 1).min(len - 1),
            Some(false) => i.saturating_sub(1),
        };
        let flip = |v: bool| dir.unwrap_or(!v);
        let before = (self.prefs.json(), self.state().profile().user.preferred_engine.clone());
        let p = &mut self.prefs;
        match pref {
            Pref::Resolution => {
                let heights = [720, 1080, 1440, 2160];
                let i = step(heights.iter().position(|h| *h == p.height).unwrap_or(3), 4);
                p.height = heights[i];
                p.width = [1280, 1920, 2560, 3840][i];
            }
            Pref::FrameRate => {
                p.fps = [60, 120][step(usize::from(p.fps > 60), 2)];
                if p.fps > 60 && p.codec == Some(Codec::Av1) {
                    p.codec = None;
                }
            }
            Pref::Hdr => p.hdr = flip(p.hdr),
            Pref::Bitrate => {
                let (min, max, by) = (uplink::BITRATE_MIN_MBPS, uplink::BITRATE_MAX_MBPS, uplink::BITRATE_STEP_MBPS);
                p.bitrate_mbps = match dir {
                    Some(false) => p.bitrate_mbps.saturating_sub(by).max(min),
                    None if p.bitrate_mbps >= max => min,
                    _ => (p.bitrate_mbps + by).min(max),
                };
            }
            Pref::Codec => {
                let mut codecs = vec![None, Some(Codec::H265)];
                if p.fps <= 60 {
                    codecs.push(Some(Codec::Av1));
                }
                codecs.push(Some(Codec::H264));
                p.codec = codecs[step(codecs.iter().position(|c| *c == p.codec).unwrap_or(0), codecs.len())];
            }
            Pref::Pacing => p.smooth_pacing = step(usize::from(!p.smooth_pacing), 2) == 0,
            Pref::Preferred => {
                let mut ids = vec![None];
                ids.extend(self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).map(|e| Some(e.manifest.id.clone())));
                let at = ids.iter().position(|id| *id == self.state.profile().user.preferred_engine).unwrap_or(0);
                self.state.profile_mut().user.preferred_engine = ids[step(at, ids.len())].clone();
            }
            Pref::Stats => {
                let (was, mode) = (p.stats.mode, StatsMode::ALL[step(p.stats.mode.index(), 3)]);
                self.set_stats(mode, out);
                return mode != was;
            }
            Pref::Light => p.light = flip(p.light),
            Pref::Covers => p.covers = Density::ALL[step(p.covers.index(), 3)],
            Pref::Text => p.text_percent = [100, 150, 200][step(usize::from((p.text_percent - 100) / 50), 3)],
            Pref::Motion => p.reduce_motion = flip(p.reduce_motion),
            Pref::Labels => p.button_labels = Labels::ALL[step(p.button_labels.index(), Labels::ALL.len())],
            Pref::GamePages => p.game_pages = flip(p.game_pages),
            Pref::MaturePages => p.mature_pages = flip(p.mature_pages),
        }
        if (self.prefs.json(), self.state().profile().user.preferred_engine.clone()) == before {
            return false;
        }
        self.save_prefs(out);
        if matches!(pref, Pref::Resolution | Pref::FrameRate | Pref::Hdr | Pref::Bitrate | Pref::Codec | Pref::Pacing | Pref::Preferred) {
            self.push_common(out);
        }
        true
    }
    fn settings_hints(&self) -> Hints {
        if self.view.top() == Some(Layer::Delete) {
            return Hints(vec![(Key::Ok, "Choose"), (Key::Back, "Keep")]);
        }
        let ok = match self.view.focus {
            Focus::Rail(_) => Some("Open"),
            Focus::Theme(_) => Some("Choose"),
            Focus::Lab(_) => Some("Start"),
            Focus::Setting(_) if matches!(self.view.pane, Some(Pane::Lab | Pane::GameSir | Pane::Leadjoy)) => Some("Change"),
            Focus::Setting(i) => self.settings_sheet().item(i).and_then(|item| match (&item.shows, &item.act) {
                (_, Act::None) => None,
                (Shows::Switch(on), _) => Some(if *on { "Turn off" } else { "Turn on" }),
                (Shows::Stepper(_), _) => Some("Change"),
                (_, Act::SignOut) => Some(if self.view.selected_target.is_some() { "Remove" } else { "Sign out" }),
                _ => Some("Open"),
            }),
            _ => None,
        };
        let mut hints: Vec<(Key, &'static str)> = ok.map(|word| (Key::Ok, word)).into_iter().collect();
        hints.push((Key::Back, "Back"));
        Hints(hints)
    }
    /// The diagnostics place's text fields: the server, the topic and the token.
    pub(super) fn save_diagnostics_text(&mut self, out: &mut Vec<Command>) {
        match self.view.edit_field {
            Some(2) => self.diagnostics.server = self.view.address.trim().into(),
            Some(3) => self.diagnostics.topic = self.view.address.trim().into(),
            Some(4) if !self.view.address.is_empty() => self.diagnostics.token = std::mem::take(&mut self.view.address),
            _ => {}
        }
        self.view.address.clear();
        out.push(Command::Diagnostics(self.diagnostics.clone()));
        self.stats_request(out);
        self.pop_layer();
    }
    /// A Leadjoy profile's name.
    pub(super) fn save_leadjoy_text(&mut self, out: &mut Vec<Command>) {
        if self.leadjoy.pending || self.leadjoy.error.is_some() { return; }
        let LeadjoyState::Ready(profile) = &self.leadjoy.state else { return };
        let Some(field) = self.view.edit_field.and_then(|i| profile.schema.fields().nth(i)) else { return };
        let value = Value::Text(self.view.address.clone());
        if !matches!(field.kind, FieldKind::Text { .. }) || field.kind.check(&value).is_err() {
            self.view.notice = Some("The name is too long. Shorten it before saving.".into());
            return;
        }
        out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(ControllerChange { setting: field.key.clone().into(), value: serde_json::json!(value) }) });
        self.leadjoy.pending = true;
        self.pop_layer();
    }
    /// How the shell should read the pads: the lab's own modes while its page has them and no sheet covers it.
    pub fn pad_mode(&self) -> PadMode {
        if !self.wants_pads() || self.view.page != Page::Settings || self.view.pane != Some(Pane::Lab) || !self.view.layers.is_empty() {
            PadMode::Menu
        } else if self.lab.capture.is_some() {
            PadMode::Captured
        } else {
            PadMode::Lab
        }
    }
    /// The kernel's stamp of the press the flash test shows lit, and the number of the frame that shows it.
    pub fn flash_press(&self) -> Option<(u64, u64)> {
        if self.pad_mode() != PadMode::Captured || self.lab.capture != Some(Capture::Flash) {
            return None;
        }
        self.lab.flash.map(|press| (press, self.lab.frame))
    }
    /// The running session's engine, when it can time the stream's round trip for the lab.
    pub fn round_trip_engine(&self) -> Option<&EngineId> {
        let s = self.session.as_ref().filter(|s| !s.is_done())?;
        let d = self.integration(&s.source.engine)?.descriptor.as_ref()?;
        d.capabilities.features.contains(&Feature::RoundTrip).then_some(&s.source.engine)
    }
    /// The lab's share of the pads. True when its page shows something new.
    pub(super) fn lab_input(&mut self, now_ns: u64, reports: Vec<(u8, RawReport)>, names: [Option<String>; MAX_PADS], frame: u64) -> bool {
        if self.view.page != Page::Settings || self.view.pane != Some(Pane::Lab) {
            return false;
        }
        let lab = &mut self.lab;
        let mut changed = false;
        for (pad, name) in lab.pads.iter_mut().zip(names) {
            if pad.as_ref().map(|p| &p.name) != name.as_ref() {
                *pad = name.map(|name| LabPad { name, ..LabPad::default() });
                changed = true;
            }
        }
        if lab.active.is_some_and(|i| lab.pads[i].is_none()) {
            lab.active = None;
        }
        let mut new_flash = false;
        for (slot, report) in reports {
            let Some(pad) = lab.pads.get_mut(usize::from(slot)).and_then(Option::as_mut) else {
                continue;
            };
            if lab.capture == Some(Capture::Flash) && lab.flash.is_none() && report.buttons & !pad.report.buttons != 0 {
                lab.flash = Some(report.kernel_ns);
                new_flash = true;
            }
            pad.b_down = if report.buttons & button::B == 0 { None } else { pad.b_down.or(Some(report.kernel_ns)) };
            pad.timing.push(report.kernel_ns);
            pad.drift[0].report(report.left, report.kernel_ns);
            pad.drift[1].report(report.right, report.kernel_ns);
            pad.report = report;
            lab.active = Some(usize::from(slot));
            changed = true;
        }
        if !new_flash && lab.frame != frame && lab.flash.is_some() && lab.pads.iter().flatten().all(|p| p.report.buttons == 0) {
            lab.flash = None;
        }
        for pad in lab.pads.iter_mut().flatten() {
            for drift in &mut pad.drift {
                changed |= drift.tick(now_ns);
            }
        }
        if now_ns >= lab.refreshed_ns + LAB_REFRESH_NS {
            lab.refreshed_ns = now_ns;
            for pad in lab.pads.iter_mut().flatten() {
                let summary = pad.timing.summary(now_ns);
                changed |= summary != pad.summary;
                pad.summary = summary;
            }
        }
        if lab.capture.is_some() && lab.pads.iter().flatten().any(|p| p.b_down.is_some_and(|at| now_ns.saturating_sub(at) >= LEAVE_HOLD_NS)) {
            lab.capture = None;
            lab.flash = None;
            changed = true;
        }
        if lab.capture == Some(Capture::Flash) && lab.frame != frame {
            lab.frame = frame;
            changed = true;
        }
        changed
    }
    fn lab_action(&mut self, action: usize, out: &mut Vec<Command>) {
        match action {
            0 => self.lab.capture = Some(Capture::Buttons),
            1 | 2 => {
                // The left motor is the heavy, low one.
                let (low, high) = if action == 1 { (0xC000, 0) } else { (0, 0xC000) };
                let slots = (0..MAX_PADS).filter(|&i| self.lab.pads[i].is_some()).map(|i| i as u8);
                let before = out.len();
                out.extend(slots.map(|slot| Command::Rumble { slot, low, high }));
                if out.len() == before {
                    self.view.notice = Some("No controller is connected.".into());
                }
            }
            3 => {
                self.lab.capture = Some(Capture::Flash);
                self.lab.flash = None;
            }
            4 => {
                if let (Some(engine), Some(session)) = (self.round_trip_engine().cloned(), self.session.as_ref().map(|s| s.id)) {
                    self.lab.trip = Trip::Waiting(engine.clone());
                    Self::send(out, &engine, Request::RoundTrip { session });
                }
            }
            _ => {}
        }
    }
    fn read_gamesir(&mut self, out: &mut Vec<Command>) {
        if !self.gamesir.pending {
            self.gamesir.pending = true;
            self.gamesir.error = None;
            out.push(Command::GameSir(None));
        }
    }
    fn adjust_gamesir(&mut self, row: usize, right: bool, out: &mut Vec<Command>) -> bool {
        if self.gamesir.pending { return true; }
        if row == 0 {
            self.read_gamesir(out);
            return true;
        }
        if self.gamesir.error.is_some() { return true; }
        let GameSirState::Ready(profile) = &self.gamesir.state else { return false };
        let values = &profile.values;
        let nudge = |value: u8, max| if right { value.saturating_add(1).min(max) } else { value.saturating_sub(1) };
        let (setting, value, current) = match row {
            1 => {
                let rates = [250, 500, 1000];
                let at = values.report_rate.and_then(|rate| rates.iter().position(|r| *r == rate));
                let next = at.map_or(0, |at| if right { (at + 1).min(2) } else { at.saturating_sub(1) });
                ("report-rate", serde_json::json!(rates[next]), serde_json::json!(values.report_rate))
            }
            2 => ("rumble-left", serde_json::json!(nudge(values.rumble_left, 4)), serde_json::json!(values.rumble_left)),
            3 => ("rumble-right", serde_json::json!(nudge(values.rumble_right, 4)), serde_json::json!(values.rumble_right)),
            4 => ("dpad-diagonal-lock", serde_json::json!(!values.dpad_diagonal_lock), serde_json::json!(values.dpad_diagonal_lock)),
            5 => ("xinput-abxy-swap", serde_json::json!(!values.xinput_abxy_swap), serde_json::json!(values.xinput_abxy_swap)),
            6 => ("deadzone-left", serde_json::json!(nudge(values.deadzone_left, 20)), serde_json::json!(values.deadzone_left)),
            7 => ("deadzone-right", serde_json::json!(nudge(values.deadzone_right, 20)), serde_json::json!(values.deadzone_right)),
            _ => return false,
        };
        if value != current {
            self.gamesir.pending = true;
            out.push(Command::GameSir(Some(ControllerChange { setting: setting.into(), value })));
        }
        true
    }
    fn read_leadjoy(&mut self, out: &mut Vec<Command>) {
        if !self.leadjoy.pending {
            self.leadjoy.pending = true;
            self.leadjoy.error = None;
            out.push(Command::Leadjoy { view: self.leadjoy.view.clone(), change: None });
        }
    }
    fn adjust_leadjoy(&mut self, row: usize, right: bool, out: &mut Vec<Command>) -> bool {
        if self.leadjoy.pending { return true; }
        if row == 0 { self.read_leadjoy(out); return true; }
        if self.leadjoy.error.is_some() { return true; }
        let LeadjoyState::Ready(profile) = &self.leadjoy.state else { return false };
        let Some(field) = profile.schema.fields().nth(row - 1) else { return false };
        if field.kind == FieldKind::Action {
            if field.key.starts_with("delete-") {
                self.view.edit_field = Some(row - 1);
                self.push_layer(Layer::Delete, Focus::SheetAction(0));
                return true;
            }
            let change = ControllerChange { setting: field.key.clone().into(), value: serde_json::Value::Null };
            self.leadjoy.pending = true;
            out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(change) });
            return true;
        }
        let Some(current) = profile.values.get(&field.key) else { return false };
        if matches!(field.kind, FieldKind::Text { .. }) {
            let Value::Text(value) = current else { return false };
            self.view.address = value.clone();
            self.view.edit_field = Some(row - 1);
            self.push_layer(Layer::EditText, Focus::Keyboard(0));
            return true;
        }
        let next = match (&field.kind, current) {
            (FieldKind::Number { min, max, step, .. }, Value::Int(v)) => Value::Int(if right { v.saturating_add(*step).min(*max) } else { v.saturating_sub(*step).max(*min) }),
            (FieldKind::Toggle { .. }, Value::Bool(v)) => Value::Bool(!v),
            (FieldKind::Choice { options, .. }, Value::Text(v)) => {
                let Some(at) = options.iter().position(|o| &o.id == v) else { return false };
                let next = if right { (at + 1).min(options.len() - 1) } else { at.saturating_sub(1) };
                Value::Text(options[next].id.clone())
            }
            _ => return false,
        };
        if next != *current && field.kind.check(&next).is_ok() {
            if let Some(selector) = field.key.strip_prefix("view-") {
                let value = match next { Value::Text(value) => value, Value::Int(value) => value.to_string(), _ => return false };
                let mut view = profile.view.clone();
                match selector {
                    "section" => { view.section = value; view.button = 1; view.step = 1; view.page = 1; }
                    "profile" => { let Ok(n) = value.parse() else { return false }; view.profile = n; view.page = 1; }
                    "sub" => { let Ok(n) = value.parse() else { return false }; view.sub = n; view.page = 1; }
                    "side" => { let Ok(n) = value.parse() else { return false }; view.side = n; }
                    "button" => { let Ok(n) = value.parse() else { return false }; view.button = n; view.step = 1; }
                    "step" => { let Ok(n) = value.parse() else { return false }; view.step = n; }
                    "action" => { let Ok(n) = value.parse() else { return false }; view.action = n; }
                    "page" => { let Ok(n) = value.parse() else { return false }; view.page = n; }
                    _ => return false,
                }
                self.leadjoy.view = view;
                self.read_leadjoy(out);
            } else {
                let change = ControllerChange { setting: field.key.clone().into(), value: serde_json::json!(next) };
                self.leadjoy.pending = true;
                out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(change) });
            }
        }
        true
    }
    fn delete_leadjoy(&mut self, out: &mut Vec<Command>) {
        if self.leadjoy.pending || self.leadjoy.error.is_some() { return; }
        let LeadjoyState::Ready(profile) = &self.leadjoy.state else { return };
        let Some(field) = self.view.edit_field.and_then(|i| profile.schema.fields().nth(i)).filter(|field| field.key.starts_with("delete-")) else { return };
        out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(ControllerChange { setting: field.key.clone().into(), value: serde_json::Value::Null }) });
        self.leadjoy.pending = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    #[test]
    fn stream_settings_limit_av1_and_bitrate_without_changing_other_fields() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.page = Page::Settings;
        m.view.place = Place::Picture;
        m.prefs.codec = Some(Codec::Av1);
        m.view.focus = Focus::Setting(1);
        key(&mut m, Key::Right);
        assert_eq!(m.prefs.fps, 120);
        assert_eq!(m.prefs.codec, None);
        m.view.focus = Focus::Setting(3);
        m.prefs.bitrate_mbps = 150;
        key(&mut m, Key::Right);
        assert_eq!(m.prefs.bitrate_mbps, 150);
    }
    #[test]
    fn frame_pacing_reaches_the_engines_and_survives_a_restart() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.page = Page::Settings;
        m.view.place = Place::Picture;
        m.view.focus = Focus::Setting(5);
        assert!(m.prefs.smooth_pacing);
        key(&mut m, Key::Right);
        assert!(!m.prefs.smooth_pacing);
        assert_eq!(m.common_values().get("frame-pacing"), Some(&Value::Text("latency".into())));
        assert!(!Preferences::read(&m.prefs.json()).smooth_pacing);
    }
    #[test]
    fn the_log_level_row_steps_between_levels_and_the_shell_hears_each_choice() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.page = Page::Settings;
        (m.view.place, m.view.pane) = (Place::System, Some(Pane::Diagnostics));
        m.view.focus = Focus::Setting(1);
        let heard = |m: &mut Model, key: Key| {
            let mut commands = m.step(Input::Key { key, down: true });
            commands.extend(m.step(Input::Key { key, down: false }));
            commands.into_iter().rev().find_map(|command| if let Command::Diagnostics(config) = command { Some(config.level) } else { None })
        };
        assert_eq!(m.diagnostics.level, DiagnosticsLevel::Info);
        assert_eq!(heard(&mut m, Key::Right), Some(DiagnosticsLevel::Debug));
        assert_eq!(heard(&mut m, Key::Right), Some(DiagnosticsLevel::Trace));
        assert_eq!(heard(&mut m, Key::Right), None, "no step past the last");
        assert_eq!(heard(&mut m, Key::Ok), Some(DiagnosticsLevel::Error), "OK goes round");
    }
    #[test]
    fn steam_page_switches_are_saved_and_mature_pages_start_off() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[]);
        engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked("main")] });
        m.open_settings(Place::Accounts);
        m.open_service(EngineId::new("steam").unwrap(), None, &mut Vec::new());
        let sheet = m.settings_sheet();
        let row = |pref| sheet.rows().into_iter().find(|&i| sheet.item(i).unwrap().act == Act::Pref(pref)).unwrap();
        assert!(m.prefs.game_pages && !m.prefs.mature_pages);
        m.view.focus = Focus::Setting(row(Pref::GamePages));
        m.rebuild_navigation();
        key(&mut m, Key::Ok);
        m.view.focus = Focus::Setting(row(Pref::MaturePages));
        key(&mut m, Key::Right);
        let saved = Preferences::read(&m.prefs.json());
        assert!(!saved.game_pages && saved.mature_pages);
        assert!(key(&mut m, Key::Right).is_empty(), "a switch already on stays on");
    }
    #[test]
    fn the_game_recorder_page_is_one_for_every_integration_and_starts_at_the_nas() {
        use lumagame_core::recorder;
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![], &[Feature::Recorder]);
        m.integrations[0].schema = SettingsSchema { sections: vec![recorder::section()] };
        m.open_settings(Place::Picture);
        let sheet = m.settings_sheet();
        m.view.focus = Focus::Setting(sheet.rows().into_iter().find(|&i| sheet.item(i).unwrap().act == Act::Pane(Pane::Recorder)).unwrap());
        m.rebuild_navigation();
        key(&mut m, Key::Ok);
        assert_eq!(m.view.pane, Some(Pane::Recorder));
        let at = |key: &str| recorder::section().fields.iter().position(|f| f.key == key).unwrap();
        let row = |m: &Model, key: &str| {
            let sheet = m.settings_sheet();
            sheet.rows().into_iter().find(|&i| sheet.item(i).unwrap().act == Act::Recorder(at(key))).unwrap()
        };
        assert_eq!(m.settings_sheet().item(row(&m, recorder::URL)).unwrap().shows, Shows::Value(recorder::DEFAULT_URL.into()));
        m.view.focus = Focus::Setting(row(&m, recorder::ENABLED));
        key(&mut m, Key::Ok);
        let saved = |m: &Model, key: &str| m.state.engines[&EngineId::new("gfn").unwrap()].settings.get(key).cloned();
        assert_eq!((m.prefs.recorder.get(recorder::ENABLED), saved(&m, recorder::ENABLED)), (Some(&Value::Bool(true)), Some(Value::Bool(true))), "the engine hears the page");
        m.view.focus = Focus::Setting(row(&m, recorder::KEYFRAMES));
        key(&mut m, Key::Right);
        key(&mut m, Key::Right);
        key(&mut m, Key::Right);
        assert_eq!(saved(&m, recorder::KEYFRAMES), Some(Value::Text("save".into())), "the choices stop at the last");
        m.view.focus = Focus::Setting(row(&m, recorder::URL));
        key(&mut m, Key::Ok);
        assert_eq!((m.view.top(), m.view.address.as_str()), (Some(Layer::EditText), recorder::DEFAULT_URL));
        m.view.address = "http://nas:9".into();
        m.save_recorder_text(&mut Vec::new());
        assert_eq!((m.view.top(), saved(&m, recorder::URL)), (None, Some(Value::Text("http://nas:9".into()))));
        assert_eq!(Preferences::read(&m.prefs.json()).recorder, m.prefs.recorder, "and it survives a restart");
    }
    #[test]
    fn a_profile_from_before_the_game_recorder_page_keeps_its_first_integrations_settings() {
        let mut state = State::default();
        state.engines.entry(EngineId::new("gfn").unwrap()).or_default().settings =
            Values::from([("recorder".into(), Value::Bool(true)), ("recorder-url".into(), Value::Text("http://nas:9".into())), ("width".into(), Value::Int(1280))]);
        let m = Model::new(state, Listing::default(), vec![]);
        let values = m.recorder_values();
        assert_eq!((values.get("recorder"), values.get("recorder-url")), (Some(&Value::Bool(true)), Some(&Value::Text("http://nas:9".into()))));
        assert!(!m.prefs.recorder.contains_key("width"), "only the recorder's keys move");
    }
    mod lab_tests {
        use super::*;

        fn lab() -> Model {
            let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
            m.view.page = Page::Settings;
            m.view.place = Place::Controllers;
            m.view.focus = Focus::Setting(0);
            key(&mut m, Key::Ok);
            m
        }
        fn reports(m: &mut Model, now_ns: u64, frame: u64, reports: Vec<(u8, RawReport)>) {
            m.step(Input::Lab { now_ns, frame, reports, names: [Some("Cyclone 2".into()), None, None, None] });
        }
        fn report(kernel_ns: u64, buttons: u16) -> (u8, RawReport) {
            (0, RawReport { kernel_ns, read_ns: kernel_ns + 100, buttons, ..Default::default() })
        }
        fn session(m: &mut Model, features: &[Feature]) {
            describe(m, "gfn", vec![], features);
            m.session = Some(Session::start(SessionId(1), GameId(1), "gfn|main|1".parse().unwrap(), Presentation::Feeder, false, 0).0);
            m.rebuild_navigation();
        }

        #[test]
        fn controllers_open_the_lab_and_back_returns_to_the_same_row() {
            let mut m = lab();
            assert_eq!((m.view.pane, m.view.focus, m.pad_mode()), (Some(Pane::Lab), Focus::Lab(0), PadMode::Lab));
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.pane, m.view.place, m.view.focus), (Page::Settings, None, Place::Controllers, Focus::Setting(0)));
            m.view.focus = Focus::Setting(1);
            assert!(key(&mut m, Key::Ok).contains(&Command::OpenControllerSettings));
        }

        #[test]
        fn capture_ignores_keys_and_remote_back_exits_on_release() {
            let mut m = lab();
            key(&mut m, Key::Ok);
            assert_eq!(m.pad_mode(), PadMode::Captured);
            let focus = m.view.focus;
            for k in [Key::Up, Key::Right, Key::Ok, Key::Guide] { key(&mut m, k); }
            assert_eq!(m.view.focus, focus);
            assert_eq!(m.lab.capture, Some(Capture::Buttons));
            m.step(Input::Key { key: Key::Back, down: true });
            assert_eq!(m.pad_mode(), PadMode::Captured);
            m.step(Input::Key { key: Key::Back, down: false });
            assert_eq!(m.pad_mode(), PadMode::Lab);
            m.view.push(Layer::Delete, m.view.focus);
            assert_eq!(m.pad_mode(), PadMode::Menu);
            m.view.clear_layers();
            m.step(Input::Foreground(false));
            assert_eq!(m.pad_mode(), PadMode::Menu);
        }

        #[test]
        fn b_hold_exits_at_one_second_without_another_report() {
            let mut m = lab();
            key(&mut m, Key::Ok);
            reports(&mut m, 10, 1, vec![report(10, button::B)]);
            reports(&mut m, 10 + LEAVE_HOLD_NS - 1, 2, vec![]);
            assert_eq!(m.lab.capture, Some(Capture::Buttons));
            reports(&mut m, 10 + LEAVE_HOLD_NS, 3, vec![]);
            assert_eq!(m.lab.capture, None);
        }

        #[test]
        fn flash_uses_the_kernel_edge_and_survives_a_batched_release_for_one_frame() {
            let mut m = lab();
            m.view.focus = Focus::Lab(3);
            key(&mut m, Key::Ok);
            reports(&mut m, 20, 7, vec![report(10, button::A), report(15, 0)]);
            assert_eq!(m.flash_press(), Some((10, 7)));
            reports(&mut m, 21, 7, vec![]);
            assert_eq!(m.flash_press(), Some((10, 7)), "no buffer swap yet");
            reports(&mut m, 22, 8, vec![]);
            assert_eq!(m.flash_press(), None);
            reports(&mut m, 30, 9, vec![report(25, button::X)]);
            assert_eq!(m.flash_press(), Some((25, 9)));
            reports(&mut m, 40, 10, vec![report(35, 0)]);
            assert_eq!(m.flash_press(), None);
        }

        #[test]
        fn rumble_tests_target_connected_pads_and_the_selected_motor() {
            let mut m = lab();
            reports(&mut m, 1, 1, vec![]);
            m.view.focus = Focus::Lab(1);
            assert!(key(&mut m, Key::Ok).contains(&Command::Rumble { slot: 0, low: 0xc000, high: 0 }));
            m.view.focus = Focus::Lab(2);
            assert!(key(&mut m, Key::Ok).contains(&Command::Rumble { slot: 0, low: 0, high: 0xc000 }));
        }

        #[test]
        fn stick_choices_are_saved_and_sent_only_under_the_common_keys() {
            let mut m = lab();
            describe(&mut m, "gfn", vec![], &[]);
            m.integrations[0].schema = serde_json::from_value(serde_json::json!({"sections":[{"title":"Sticks","fields":[
                {"key":"deadzone","label":"Dead zone","type":"number","min":0,"max":20,"step":1,"default":8},
                {"key":"anti-deadzone","label":"Anti-deadzone","type":"number","min":0,"max":30,"step":1,"default":0},
                {"key":"stick-curve","label":"Curve","type":"choice","options":[{"id":"linear","label":"Linear"},{"id":"precise","label":"Precise"},{"id":"quick","label":"Quick"}],"default":"linear"}
            ]}]})).unwrap();
            for row in 0..3 {
                m.view.focus = Focus::Setting(row);
                let commands = m.step(Input::Key { key: Key::Right, down: true });
                assert!(commands.contains(&Command::SaveState));
                let Request::Settings { values } = sent(&commands)[0] else { panic!("settings command") };
                assert_eq!(values.len(), 3);
            }
            assert_eq!((m.prefs.deadzone, m.prefs.anti_deadzone, m.prefs.stick_curve), (9, 1, Curve::Precise));
            assert_eq!(Preferences::read(&m.state.profile().ui), m.prefs);
            let values = &m.state.engines[&EngineId::new("gfn").unwrap()].settings;
            assert_eq!(values["deadzone"], Value::Int(9));
            assert_eq!(values["anti-deadzone"], Value::Int(1));
            assert_eq!(values["stick-curve"], Value::Text("precise".into()));
        }

        #[test]
        fn an_engines_background_error_stays_on_its_card_and_off_the_lab() {
            let mut m = lab();
            let engine = EngineId::new("gfn").unwrap();
            let error = |m: &mut Model| m.step(Input::Engine { engine: engine.clone(), message: Box::new(FromEngine { re: Some(1), event: ipc::Event::Error {
                code: ipc::ErrorCode::Internal, message: "the installed app list exceeds the lifecycle limit".into() } }) });
            error(&mut m);
            assert_eq!(m.view.pane, Some(Pane::Lab));
            assert_eq!(m.view.notice, None);
            assert_eq!(m.integration(&engine).unwrap().error.as_ref().map(|(_, text)| text.as_str()), Some("the installed app list exceeds the lifecycle limit"));
            // Where the engine is what the screen is about, the same error is still shown.
            m.view.notice = Some("No controller is connected.".into());
            m.view.selected_engine = Some(engine.clone());
            m.view.push(Layer::AddHost, m.view.focus);
            error(&mut m);
            assert_eq!(m.view.notice.as_deref(), Some("the installed app list exceeds the lifecycle limit"));
            // A notice does not follow its page to another one.
            m.view.clear_layers();
            m.show_page(Page::Settings, Focus::Setting(0));
            assert_eq!(m.view.notice, None);
        }

        #[test]
        fn round_trip_is_additive_and_old_engines_have_no_measure_action() {
            let mut m = lab();
            session(&mut m, &[]);
            assert_eq!(m.pad_mode(), PadMode::Menu);
            assert_eq!(m.round_trip_engine(), None);
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Lab(4)));
            let mut out = Vec::new();
            m.lab_action(4, &mut out);
            assert!(out.is_empty());
            session(&mut m, &[Feature::RoundTrip]);
            m.view.focus = Focus::Lab(4);
            let commands = key(&mut m, Key::Ok);
            assert!(matches!(sent(&commands).as_slice(), [Request::RoundTrip { session: SessionId(1) }]));
            let engine = EngineId::new("gfn").unwrap();
            let trip = ipc::RoundTrip { network_ms: Some(12.5), ..Default::default() };
            m.step(Input::Engine { engine: engine.clone(), message: Box::new(FromEngine { re: None, event: ipc::Event::RoundTrip { session: SessionId(1), trip: trip.clone() } }) });
            assert_eq!(m.lab.trip, Trip::Answer(engine.clone(), trip));
            key(&mut m, Key::Ok);
            m.step(Input::Engine { engine: engine.clone(), message: Box::new(FromEngine { re: None, event: ipc::Event::Error { code: ipc::ErrorCode::Unsupported, message: "unknown request".into() } }) });
            assert_eq!(m.lab.trip, Trip::Unsupported(engine));
            assert_eq!(m.view.notice, None);
            assert_eq!(m.view.focus, Focus::Setting(0));
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Lab(4)));
        }

        #[test]
        fn gamesir_waits_for_readback_and_keeps_last_confirmed_values_on_error() {
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(2);
            assert!(key(&mut m, Key::Ok).contains(&Command::GameSir(None)));
            assert_eq!(m.view.pane, Some(Pane::GameSir));
            assert!(m.gamesir.pending);
            let ready = crate::game::helper::gamesir_fixture();
            m.step(Input::GameSir(Ok(ready.clone())));
            m.view.focus = Focus::Setting(1);
            let commands = m.step(Input::Key { key: Key::Right, down: true });
            assert!(commands.contains(&Command::GameSir(Some(ControllerChange { setting: "report-rate".into(), value: serde_json::json!(1000) }))));
            assert_eq!(m.gamesir.state, ready, "no optimistic hardware value");
            assert!(m.step(Input::Key { key: Key::Right, down: true }).is_empty(), "one request in flight");
            m.step(Input::GameSir(Err("Read-back did not match".into())));
            assert_eq!(m.gamesir.state, ready);
            assert!(!m.gamesir.pending);
            assert_eq!(m.view.focus, Focus::Setting(0), "the error is visible beside Refresh");
            m.view.focus = Focus::Setting(1);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).is_empty(), "refresh is required after a failed write");
            m.view.focus = Focus::Setting(0);
            assert!(key(&mut m, Key::Ok).contains(&Command::GameSir(None)));
            m.step(Input::GameSir(Ok(GameSirState::Mode { device: "Wireless Controller".into() })));
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Setting(1)));
            m.step(Input::GameSir(Ok(GameSirState::Off)));
            assert_eq!(m.gamesir.state, GameSirState::Off);
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.pane, m.view.focus), (Page::Settings, None, Focus::Setting(2)));
        }

        #[test]
        fn leadjoy_edits_wait_for_readback_and_keep_refresh_available() {
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: None }));
            assert_eq!(m.view.pane, Some(Pane::Leadjoy));
            let ready = crate::game::helper::leadjoy_fixture("general");
            m.step(Input::Leadjoy(Ok(ready.clone())));
            m.view.focus = Focus::Setting(4);
            assert!(m.step(Input::Key { key: Key::Left, down: true }).is_empty(), "the green channel is already zero");
            m.view.focus = Focus::Setting(2);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: Some(ControllerChange {
                setting: "logo-brightness".into(), value: serde_json::json!(81),
            }) }));
            assert_eq!(m.leadjoy.state, ready);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).is_empty());
            m.step(Input::Leadjoy(Err("Read-back did not match".into())));
            assert_eq!(m.leadjoy.state, ready);
            assert_eq!(m.view.focus, Focus::Setting(0));
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: None }));
            m.step(Input::Leadjoy(Ok(LeadjoyState::Unsupported { device: "Other".into(), firmware: "1".into() })));
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Setting(1)));
            m.step(Input::Leadjoy(Ok(LeadjoyState::Off)));
            assert!(!m.leadjoy.pending);
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.pane, m.view.focus), (Page::Settings, None, Focus::Setting(3)));
        }

        #[test]
        fn leadjoy_selectors_reread_while_names_and_actions_go_to_the_helper() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            key(&mut m, Key::Ok);
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("general"))));
            m.view.focus = Focus::Setting(1);
            let profiles = LeadjoyView { section: "profiles".into(), ..LeadjoyView::default() };
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: profiles.clone(), change: None }));
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("profiles"))));
            m.view.focus = Focus::Setting(4);
            assert!(key(&mut m, Key::Ok).is_empty());
            assert_eq!((m.view.top(), m.view.address.as_str()), (Some(Layer::EditText), "Default"));
            m.view.address = "Racing".into();
            m.view.focus = Focus::SheetAction(0);
            let rename = ControllerChange { setting: "profile-name".into(), value: serde_json::json!("Racing") };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: profiles.clone(), change: Some(rename) }));
            assert_eq!(m.view.top(), None);
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("profiles"))));
            m.view.focus = Focus::Setting(5);
            let activate = ControllerChange { setting: "profile-activate".into(), value: serde_json::Value::Null };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: profiles, change: Some(activate) }));
        }

        #[test]
        fn leadjoy_macro_steps_reread_and_deleting_needs_confirmation() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            key(&mut m, Key::Ok);
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("macros"))));
            let macros = LeadjoyView { section: "macros".into(), ..LeadjoyView::default() };
            m.view.focus = Focus::Setting(3);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: LeadjoyView { step: 2, ..macros.clone() }, change: None }));
            let LeadjoyState::Ready(mut second) = leadjoy_fixture("macros") else { panic!("the fixture is ready") };
            second.view.step = 2;
            m.step(Input::Leadjoy(Ok(LeadjoyState::Ready(second))));
            m.view.focus = Focus::Setting(2);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: LeadjoyView { button: 2, ..macros.clone() }, change: None }), "another macro starts at its first step");
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("macros"))));
            m.view.focus = Focus::Setting(11);
            assert!(key(&mut m, Key::Ok).is_empty());
            assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Delete), Focus::SheetAction(0)));
            assert!(key(&mut m, Key::Ok).is_empty(), "keeping is the default");
            assert_eq!((m.view.top(), m.view.focus), (None, Focus::Setting(11)));
            key(&mut m, Key::Ok);
            m.view.focus = Focus::SheetAction(1);
            let delete = ControllerChange { setting: "delete-macro".into(), value: serde_json::Value::Null };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: macros, change: Some(delete) }));
            assert_eq!((m.view.top(), m.leadjoy.pending), (None, true));
        }

        #[test]
        fn leadjoy_keyboard_wraps_and_keeps_the_selected_field_after_readback() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            m.view.pane = Some(Pane::Leadjoy);
            let ready = leadjoy_fixture("keyboard");
            m.step(Input::Leadjoy(Ok(ready.clone())));
            let LeadjoyState::Ready(profile) = &ready else { panic!("keyboard fixture must be ready") };
            let row = profile.schema.fields().position(|field| field.key == "key-0").unwrap() + 1;
            m.view.focus = Focus::Setting(row);
            let change = ControllerChange { setting: "key-0".into(), value: serde_json::json!("227") };
            assert!(m.step(Input::Key { key:Key::Left, down:true }).contains(&Command::Leadjoy { view:profile.view.clone(), change:Some(change) }));

            let LeadjoyState::Ready(mut readback) = ready else { unreachable!() };
            let wrapped = readback.schema.fields().find(|field| field.key == "key-1").unwrap().kind.clone();
            let first = readback.schema.sections[0].fields.iter_mut().find(|field| field.key == "key-0").unwrap();
            first.kind = wrapped;
            readback.values.insert("key-0".into(),Value::Text("227".into()));
            readback.schema.sections[0].fields.insert(0, lumagame_core::settings::Field {
                key:"read-only".into(), label:"Read only".into(), help:None, kind:FieldKind::Note,
            });
            readback.schema.validate().unwrap();
            let view = readback.view.clone();
            m.step(Input::Leadjoy(Ok(LeadjoyState::Ready(readback))));
            assert_eq!(m.view.focus,Focus::Setting(row + 1));
            let change = ControllerChange { setting:"key-0".into(), value:serde_json::json!("0") };
            assert!(m.step(Input::Key { key:Key::Right, down:true }).contains(&Command::Leadjoy { view, change:Some(change) }));
        }

        #[test]
        fn leadjoy_long_key_pages_are_read_only_selectors_and_notes_are_not_controls() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            m.view.pane = Some(Pane::Leadjoy);
            let LeadjoyState::Ready(profile) = leadjoy_fixture("motion-long") else { panic!("motion fixture must be ready") };
            let page_row = profile.schema.fields().position(|field| field.key == "view-page").unwrap() + 1;
            let key_row = profile.schema.fields().position(|field| field.key == "trigger-key-47").unwrap() + 1;
            let mut next = profile.view.clone(); next.page = 3;
            m.step(Input::Leadjoy(Ok(LeadjoyState::Ready(profile))));
            assert!(m.navigation.iter().any(|entry| entry.id == Focus::Setting(key_row)));
            m.view.focus = Focus::Setting(page_row);
            assert!(m.step(Input::Key { key:Key::Right, down:true }).contains(&Command::Leadjoy { view:next, change:None }));

            m.step(Input::Leadjoy(Ok(leadjoy_fixture("sticks-missing"))));
            let LeadjoyState::Ready(profile) = &m.leadjoy.state else { unreachable!() };
            for (i, field) in profile.schema.fields().enumerate().filter(|(_, field)| matches!(field.kind, FieldKind::Note)) {
                assert!(!m.navigation.iter().any(|entry| entry.id == Focus::Setting(i + 1)), "{}", field.key);
            }
        }

        #[test]
        fn lab_navigation_reaches_every_reading_without_sending_commands() {
            let mut m = lab();
            reports(&mut m, 1_000_000_000, 1, vec![report(1_000_000_000, 0)]);
            m.prefs.text_percent = 200;
            m.view.focus = Focus::Lab(2);
            m.rebuild_navigation();
            for expected in [Focus::Lab(5), Focus::Lab(6), Focus::Lab(7), Focus::Setting(0)] {
                key(&mut m, Key::Down);
                assert_eq!(m.view.focus, expected);
                if matches!(expected, Focus::Lab(_)) { assert!(key(&mut m, Key::Ok).is_empty()); }
            }
        }
    }
}

#[cfg(test)]
mod settings_navigation_tests {
    use super::*;
    #[test]
    fn the_rail_steps_places_and_each_place_returns_to_its_row() {
        let mut m = Model::new(State::default(), Listing::default(), Vec::new());
        m.open_settings(Place::Picture);
        m.rebuild_navigation();
        let press = |m: &mut Model, key: Key| {
            m.step(Input::Key { key, down: true });
            m.step(Input::Key { key, down: false });
        };
        press(&mut m, Key::Right);
        assert_eq!(m.view.focus, Focus::Setting(0));
        press(&mut m, Key::Down);
        press(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::Setting(2));
        press(&mut m, Key::Back);
        assert_eq!(m.view.focus, Focus::Rail(1));
        press(&mut m, Key::Down);
        assert_eq!((m.view.focus, m.view.place), (Focus::Rail(2), Place::Controllers));
        press(&mut m, Key::Up);
        press(&mut m, Key::Right);
        assert_eq!((m.view.place, m.view.focus), (Place::Picture, Focus::Setting(2)));
    }
}
