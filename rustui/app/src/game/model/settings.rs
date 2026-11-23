//! Settings: the rail and its places, the theme chooser, the controllers' lab and the GameSir and Leadjoy pages.
use super::nav::Slice;
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Place {
    #[default]
    Appearance,
    Streaming,
    Integrations,
    Controllers,
    Diagnostics,
    About,
}
impl Place {
    pub const ALL: [Self; 6] = [Self::Appearance, Self::Streaming, Self::Integrations, Self::Controllers, Self::Diagnostics, Self::About];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        ["Appearance", "Streaming", "Integrations", "Controllers", "Diagnostics", "About"][self.index()]
    }
}
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
}
impl Model {
    /// Shows Settings at `place`, focused on its first control.
    pub fn open_settings(&mut self, place: Place) {
        self.view.place = place;
        self.show_page(Page::Settings, if place == Place::Integrations { Focus::Integration(0) } else { Focus::Setting(0) });
    }
    fn settings_grid(&self, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        match (layer, self.view.page) {
            (Some(Layer::Theme), _) => {
                let count = lumatv_tokens::themes::game_options().len();
                let cols = if self.prefs.text_percent > 100 { 3 } else { 4 };
                for i in (0..count).step_by(cols) {
                    rows.push((i..(i + cols).min(count)).map(Focus::Theme).collect());
                }
            }
            (Some(Layer::Delete), _) => rows.push(vec![Focus::SheetAction(0), Focus::SheetAction(1)]),
            (Some(_), _) => {}
            (None, Page::Settings) => {
                if self.view.place == Place::Appearance {
                    rows.push((1..=if self.prefs.text_percent > 100 { 2 } else { 4 }).map(Focus::Theme).collect());
                }
                let count = match self.view.place {
                    Place::Appearance => 6,
                    Place::Streaming => 8,
                    Place::Integrations => self.integrations.len(),
                    Place::Controllers => 5,
                    Place::Diagnostics => 7,
                    Place::About => 3,
                };
                for i in 0..count.max(Place::ALL.len()) {
                    let mut row = Vec::new();
                    if i < Place::ALL.len() {
                        row.push(Focus::Rail(i));
                    }
                    if i < count {
                        row.push(if self.view.place == Place::Integrations { Focus::Integration(i) } else { Focus::Setting(i) });
                    }
                    rows.push(row);
                }
            }
            (None, Page::Lab) => {
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
            (None, Page::GameSir) => {
                let count = if matches!(self.gamesir.state, GameSirState::Ready(_)) { 8 } else { 1 };
                rows.extend((0..count).map(|i| vec![Focus::Setting(i)]));
            }
            (None, Page::Leadjoy) => {
                rows.push(vec![Focus::Setting(0)]);
                if let LeadjoyState::Ready(profile) = &self.leadjoy.state {
                    rows.extend(profile.schema.fields().enumerate().filter(|(_, field)| !matches!(field.kind, FieldKind::Note))
                        .map(|(i, _)| vec![Focus::Setting(i + 1)]));
                }
            }
            (None, _) => {}
        }
        rows
    }
    /// The Settings page's rail and pane link to each other explicitly: the rail's right goes to the pane's first control, and
    /// every pane control's left returns to the place it belongs to.
    pub(super) fn settings_links(&mut self) {
        let count = self.navigation.iter().filter(|n| matches!(n.id, Focus::Setting(_) | Focus::Integration(_))).count();
        let pane = |i| if self.view.place == Place::Integrations { Focus::Integration(i) } else { Focus::Setting(i) };
        let section = self.view.place.index();
        for n in &mut self.navigation {
            match n.id {
                Focus::Rail(i) => {
                    n.left = None;
                    n.right = if self.view.place == Place::Appearance {
                        Some(Focus::Theme(1))
                    } else if count > 0 {
                        Some(pane(0))
                    } else {
                        None
                    };
                    n.up = Some(if i == 0 { Focus::Nav(3) } else { Focus::Rail(i - 1) });
                    n.down = (i + 1 < Place::ALL.len()).then_some(Focus::Rail(i + 1));
                }
                Focus::Setting(i) | Focus::Integration(i) => {
                    n.left = Some(Focus::Rail(section));
                    n.right = None;
                    n.up = Some(if i > 0 {
                        pane(i - 1)
                    } else if self.view.place == Place::Appearance {
                        Focus::Theme(1)
                    } else {
                        Focus::Nav(3)
                    });
                    n.down = (i + 1 < count).then_some(pane(i + 1));
                }
                Focus::Theme(_) => {
                    n.up = Some(Focus::Nav(3));
                    n.down = Some(Focus::Setting(0));
                }
                _ => {}
            }
        }
    }
    fn settings_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Rail(i) => {
                self.view.place = Place::ALL[i];
                self.view.focus = if self.view.place == Place::Integrations { Focus::Integration(0) } else { Focus::Setting(0) };
            }
            Focus::Setting(i) => {
                if matches!(self.view.page, Page::Lab | Page::GameSir | Page::Leadjoy) {
                    self.adjust(true, out);
                } else if self.view.place == Place::Appearance && i == 0 {
                    self.open_sheet(Layer::Theme, Focus::Theme(0));
                } else if self.view.place == Place::Controllers {
                    if i == 0 {
                        self.lab = Lab::default();
                        let focus = if !self.session_active() {
                            Focus::Lab(0)
                        } else if self.round_trip_engine().is_some() {
                            Focus::Lab(4)
                        } else {
                            Focus::Setting(0)
                        };
                        self.show_page(Page::Lab, focus);
                    } else if i == 1 {
                        out.push(Command::OpenControllerSettings);
                    } else if i == 2 {
                        self.show_page(Page::GameSir, Focus::Setting(0));
                        self.read_gamesir(out);
                    } else if i == 4 {
                        self.prefs.button_labels = Labels::ALL[(self.prefs.button_labels.index() + 1) % Labels::ALL.len()];
                        self.save_prefs(out);
                    } else {
                        self.show_page(Page::Leadjoy, Focus::Setting(0));
                        self.read_leadjoy(out);
                    }
                } else if self.view.place == Place::Diagnostics {
                    match i {
                        0 => { self.diagnostics.enabled = !self.diagnostics.enabled; out.push(Command::Diagnostics(self.diagnostics.clone())); self.stats_request(out); }
                        1 => { self.diagnostics.level = DiagnosticsLevel::ALL[(self.diagnostics.level.index() + 1) % DiagnosticsLevel::ALL.len()]; out.push(Command::Diagnostics(self.diagnostics.clone())); }
                        2..=4 => {
                            self.view.edit_field = Some(i);
                            self.view.address = match i { 2 => self.diagnostics.server.clone(), 3 => self.diagnostics.topic.clone(), _ => String::new() };
                            self.open_sheet(Layer::EditText, Focus::Keyboard(0));
                        }
                        5 => { self.diagnostics.token.clear(); out.push(Command::Diagnostics(self.diagnostics.clone())); }
                        6 => out.push(Command::DiagnosticTest),
                        _ => {}
                    }
                } else if self.view.place == Place::About {
                    out.push(Command::PlayMedia { url: ["https://store.steampowered.com", "https://steamgriddb.com", "https://igdb.com"][i.min(2)].into() });
                } else {
                    self.adjust(true, out);
                }
            }
            Focus::Lab(i) => self.lab_action(i, out),
            Focus::Theme(i) => {
                let families = lumatv_tokens::themes::game_catalog();
                self.prefs.family = if i == 0 { "match-lumatv".into() } else { families.get(i - 1).map_or("rose-pine", |f| f.id).into() };
                self.save_prefs(out);
                if self.view.sheet.is_some() {
                    self.close_sheet();
                }
            }
            Focus::Integration(i) => self.open_integration(i),
            Focus::SheetAction(i) if self.view.sheet == Some(Layer::Delete) => {
                if i == 1 { self.delete_leadjoy(out); }
                self.close_sheet();
            }
            _ => {}
        }
    }
    fn settings_adjust(&mut self, f: Focus, right: bool, out: &mut Vec<Command>) -> bool {
        let Focus::Setting(row) = f else {
            return false;
        };
        if self.view.page == Page::GameSir {
            return self.adjust_gamesir(row, right, out);
        }
        if self.view.page == Page::Leadjoy { return self.adjust_leadjoy(row, right, out); }
        let step = |index: usize, len: usize| if right { (index + 1).min(len - 1) } else { index.saturating_sub(1) };
        if self.view.page == Page::Lab {
            let nudge = |v: u8, max: u8| if right { (v + 1).min(max) } else { v.saturating_sub(1) };
            let curves = [Curve::Linear, Curve::Precise, Curve::Quick];
            match row {
                0 => self.prefs.deadzone = nudge(self.prefs.deadzone, DEADZONE_MAX),
                1 => self.prefs.anti_deadzone = nudge(self.prefs.anti_deadzone, ANTI_DEADZONE_MAX),
                2 => self.prefs.stick_curve = curves[step(curves.iter().position(|c| *c == self.prefs.stick_curve).unwrap_or(0), curves.len())],
                _ => return false,
            }
            self.save_prefs(out);
            self.push_common(out);
            return true;
        }
        match self.view.place {
            Place::Appearance => match row {
                0 => return false,
                1 => self.prefs.light = !self.prefs.light,
                2 => self.prefs.hero = Density::ALL[step(self.prefs.hero.index(), 3)],
                3 => self.prefs.covers = Density::ALL[step(self.prefs.covers.index(), 3)],
                4 => self.prefs.text_percent = [100, 150, 200][step(usize::from((self.prefs.text_percent - 100) / 50), 3)],
                5 => self.prefs.reduce_motion = !self.prefs.reduce_motion,
                _ => return false,
            },
            Place::Streaming => match row {
                0 => {
                    let heights = [720, 1080, 1440, 2160];
                    let i = step(heights.iter().position(|h| *h == self.prefs.height).unwrap_or(3), 4);
                    self.prefs.height = heights[i];
                    self.prefs.width = [1280, 1920, 2560, 3840][i];
                }
                1 => {
                    self.prefs.fps = if right { 120 } else { 60 };
                    if self.prefs.fps > 60 && self.prefs.codec == Some(Codec::Av1) {
                        self.prefs.codec = None;
                    }
                }
                2 => self.prefs.hdr = !self.prefs.hdr,
                3 => {
                    let (min, max, step) = (uplink::BITRATE_MIN_MBPS, uplink::BITRATE_MAX_MBPS, uplink::BITRATE_STEP_MBPS);
                    self.prefs.bitrate_mbps = if right { (self.prefs.bitrate_mbps + step).min(max) } else { self.prefs.bitrate_mbps.saturating_sub(step).max(min) };
                }
                4 => {
                    let mut codecs = vec![None, Some(Codec::H265)];
                    if self.prefs.fps <= 60 {
                        codecs.push(Some(Codec::Av1));
                    }
                    codecs.push(Some(Codec::H264));
                    self.prefs.codec = codecs[step(codecs.iter().position(|c| *c == self.prefs.codec).unwrap_or(0), codecs.len())];
                }
                5 => self.prefs.smooth_pacing = step(usize::from(!self.prefs.smooth_pacing), 2) == 0,
                6 => {
                    let mut ids = vec![None];
                    ids.extend(self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).map(|e| Some(e.manifest.id.clone())));
                    let at = ids.iter().position(|id| *id == self.state.profile().user.preferred_engine).unwrap_or(0);
                    self.state.profile_mut().user.preferred_engine = ids[step(at, ids.len())].clone();
                }
                7 => {
                    self.set_stats(StatsMode::ALL[step(self.prefs.stats.mode.index(), 3)], out);
                    return true;
                }
                _ => return false,
            },
            Place::Controllers => match row {
                4 => self.prefs.button_labels = Labels::ALL[step(self.prefs.button_labels.index(), Labels::ALL.len())],
                _ => return false,
            },
            Place::Diagnostics => match row {
                1 => {
                    self.diagnostics.level = DiagnosticsLevel::ALL[step(self.diagnostics.level.index(), DiagnosticsLevel::ALL.len())];
                    out.push(Command::Diagnostics(self.diagnostics.clone()));
                    return true;
                }
                _ => return false,
            },
            _ => return false,
        }
        self.save_prefs(out);
        if self.view.place == Place::Streaming {
            self.push_common(out);
        }
        true
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
        self.close_sheet();
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
        self.close_sheet();
    }
    /// How the shell should read the pads: the lab's own modes while its page has them and no sheet covers it.
    pub fn pad_mode(&self) -> PadMode {
        if !self.wants_pads() || self.view.page != Page::Lab || self.view.sheet.is_some() {
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
        if self.view.page != Page::Lab {
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
                self.open_sheet(Layer::Delete, Focus::SheetAction(0));
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
            self.open_sheet(Layer::EditText, Focus::Keyboard(0));
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
        m.view.place = Place::Streaming;
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
        m.view.place = Place::Streaming;
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
        m.view.place = Place::Diagnostics;
        m.view.focus = Focus::Setting(1);
        let heard = |m: &mut Model, key: Key| {
            let mut commands = m.step(Input::Key { key, down: true });
            commands.extend(m.step(Input::Key { key, down: false }));
            commands.into_iter().rev().find_map(|command| if let Command::Diagnostics(config) = command { Some(config.level) } else { None })
        };
        assert_eq!(m.diagnostics.level, DiagnosticsLevel::Info);
        assert_eq!(heard(&mut m, Key::Right), Some(DiagnosticsLevel::Debug));
        assert_eq!(heard(&mut m, Key::Right), Some(DiagnosticsLevel::Trace));
        assert_eq!(heard(&mut m, Key::Right), Some(DiagnosticsLevel::Trace), "no step past the last");
        assert_eq!(heard(&mut m, Key::Ok), Some(DiagnosticsLevel::Error), "OK goes round");
    }
    #[test]
    fn integrations_of_one_kind_sit_under_one_heading() {
        let m = Model::new(
            State::default(),
            Listing::default(),
            vec![manifest("gamestream", "Sunshine", EngineKind::Host), manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("punktfunk", "Punktfunk", EngineKind::Host)],
        );
        let order: Vec<_> = m.integrations.iter().map(|e| e.manifest.id.as_str()).collect();
        assert_eq!(order, ["gfn", "gamestream", "punktfunk"]);
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
            assert_eq!((m.view.page, m.view.focus, m.pad_mode()), (Page::Lab, Focus::Lab(0), PadMode::Lab));
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.place, m.view.focus), (Page::Settings, Place::Controllers, Focus::Setting(0)));
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
            m.view.sheet = Some(Layer::Theme);
            assert_eq!(m.pad_mode(), PadMode::Menu);
            m.view.sheet = None;
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
            let error = |m: &mut Model| m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: Some(1), event: ipc::Event::Error {
                code: ipc::ErrorCode::Internal, message: "the installed app list exceeds the lifecycle limit".into() } } });
            error(&mut m);
            assert_eq!(m.view.page, Page::Lab);
            assert_eq!(m.view.notice, None);
            assert_eq!(m.integration(&engine).unwrap().error.as_ref().map(|(_, text)| text.as_str()), Some("the installed app list exceeds the lifecycle limit"));
            // Where the engine is what the screen is about, the same error is still shown.
            m.view.notice = Some("No controller is connected.".into());
            m.view.selected_engine = Some(engine.clone());
            m.view.sheet = Some(Layer::AddHost);
            error(&mut m);
            assert_eq!(m.view.notice.as_deref(), Some("the installed app list exceeds the lifecycle limit"));
            // A notice does not follow its page to another one.
            m.view.sheet = None;
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
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event: ipc::Event::RoundTrip { session: SessionId(1), trip: trip.clone() } } });
            assert_eq!(m.lab.trip, Trip::Answer(engine.clone(), trip));
            key(&mut m, Key::Ok);
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event: ipc::Event::Error { code: ipc::ErrorCode::Unsupported, message: "unknown request".into() } } });
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
            assert_eq!(m.view.page, Page::GameSir);
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
            assert_eq!((m.view.page, m.view.focus), (Page::Settings, Focus::Setting(2)));
        }

        #[test]
        fn leadjoy_edits_wait_for_readback_and_keep_refresh_available() {
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: None }));
            assert_eq!(m.view.page, Page::Leadjoy);
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
            assert_eq!((m.view.page, m.view.focus), (Page::Settings, Focus::Setting(3)));
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
            assert_eq!((m.view.sheet, m.view.address.as_str()), (Some(Layer::EditText), "Default"));
            m.view.address = "Racing".into();
            m.view.focus = Focus::SheetAction(0);
            let rename = ControllerChange { setting: "profile-name".into(), value: serde_json::json!("Racing") };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: profiles.clone(), change: Some(rename) }));
            assert_eq!(m.view.sheet, None);
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
            assert_eq!((m.view.sheet, m.view.focus), (Some(Layer::Delete), Focus::SheetAction(0)));
            assert!(key(&mut m, Key::Ok).is_empty(), "keeping is the default");
            assert_eq!((m.view.sheet, m.view.focus), (None, Focus::Setting(11)));
            key(&mut m, Key::Ok);
            m.view.focus = Focus::SheetAction(1);
            let delete = ControllerChange { setting: "delete-macro".into(), value: serde_json::Value::Null };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: macros, change: Some(delete) }));
            assert_eq!((m.view.sheet, m.leadjoy.pending), (None, true));
        }

        #[test]
        fn leadjoy_keyboard_wraps_and_keeps_the_selected_field_after_readback() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            m.view.page = Page::Leadjoy;
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
            m.view.page = Page::Leadjoy;
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
    fn rails_reach_their_own_pane_and_long_panes_keep_their_column() {
        let mut m = Model::new(State::default(), Listing::default(), Vec::new());
        m.view.page = Page::Settings;
        m.view.place = Place::Controllers;
        m.view.focus = Focus::Rail(3);
        m.rebuild_navigation();
        m.step(Input::Key { key: Key::Right, down: true });
        assert_eq!(m.view.focus, Focus::Setting(0));
        assert_eq!(m.view.place, Place::Controllers);
        m.view.place = Place::Appearance;
        m.view.focus = Focus::Setting(5);
        m.rebuild_navigation();
        m.step(Input::Key { key: Key::Up, down: true });
        assert_eq!(m.view.focus, Focus::Setting(4));
    }
}
