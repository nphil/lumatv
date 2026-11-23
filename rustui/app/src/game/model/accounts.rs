//! An integration's own page: switching it on, linking an account or pairing a PC, adding a host, its targets and fields, signing
//! out, and the link flow's sheet.
use super::nav::Slice;
use super::*;

#[derive(Clone, Debug)]
pub struct LinkFlow {
    pub engine: EngineId,
    pub target: Option<TargetId>,
    pub state: Option<AuthState>,
    pub expires_ms: Option<u64>,
    pub pin: String,
    pub since_ms: u64,
    /// A PC engine searched for [`PC_SEARCH_MS`] without offering a PC to pair.
    pub no_pcs: bool,
}
/// How long "Pair a PC" waits for a PC engine to find one before offering an address and a retry.
pub const PC_SEARCH_MS: u64 = 20_000;
impl LinkFlow {
    pub(super) fn new(engine: EngineId, target: Option<TargetId>, now_ms: u64) -> Self {
        Self { engine, target, state: None, expires_ms: None, pin: String::new(), since_ms: now_ms, no_pcs: false }
    }
}
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.accounts_grid(layer)
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.accounts_activate(f, out);
    }
    fn adjust(m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool {
        m.accounts_adjust(f, forward, out)
    }
}
impl Model {
    /// Opens the page of the `i`th integration.
    pub(super) fn open_integration(&mut self, i: usize) {
        if let Some(e) = self.integrations.get(i) {
            self.view.selected_engine = Some(e.manifest.id.clone());
            self.show_page(Page::Accounts, Focus::Setting(0));
        }
    }
    /// Starts linking `engine`'s account, or pairing a PC with it.
    pub fn begin_auth(&mut self, engine: EngineId, out: &mut Vec<Command>) {
        self.begin_link(engine, None, out);
    }
    fn accounts_grid(&self, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        match layer {
            Some(Layer::Auth) if self.view.auth.as_ref().is_some_and(|a| matches!(a.state, Some(AuthState::EnterPin { .. }))) => {
                for i in (0..12).step_by(6) {
                    rows.push((i..i + 6).map(Focus::Keyboard).collect());
                }
                rows.push(vec![Focus::SheetAction(0)]);
            }
            Some(Layer::Auth) if self.view.auth.as_ref().is_some_and(|a| a.no_pcs) => {
                let manual = self.view.auth.as_ref().and_then(|a| self.integration(&a.engine)).and_then(|e| e.descriptor.as_ref()).is_some_and(|d| d.capabilities.has(Feature::ManualHost));
                rows.push(if manual { vec![Focus::SheetAction(0), Focus::SheetAction(1)] } else { vec![Focus::SheetAction(0)] });
            }
            Some(Layer::Auth) => rows.push(vec![Focus::SheetAction(0)]),
            Some(_) => {}
            None => {
                rows.push(vec![Focus::Setting(0)]);
                if let Some(e) = self.view.selected_engine.as_ref().and_then(|id| self.integration(id)).filter(|e| self.enabled(&e.manifest.id)) {
                    if e.descriptor.as_ref().is_some_and(|d| !d.capabilities.auth.is_empty()) {
                        rows.push(vec![Focus::Setting(1)]);
                    }
                    if e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::ManualHost)) {
                        rows.push(vec![Focus::Setting(2)]);
                    }
                    rows.extend((0..e.targets.len()).map(|i| vec![Focus::Target(i)]));
                    if self.can_refresh(e) {
                        rows.push(vec![Focus::Setting(4)]);
                    }
                    if e.can_sign_out() {
                        rows.push(vec![Focus::Setting(3)]);
                    }
                    rows.extend(e.schema.fields().enumerate().filter(|(_, f)| f.kind != FieldKind::Note).map(|(i, _)| vec![Focus::EngineField(i)]));
                }
            }
        }
        rows
    }
    fn accounts_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Setting(_) => {
                self.adjust(true, out);
            }
            Focus::Target(i) => {
                if let Some(id) = self.view.selected_engine.clone() {
                    let target = self.integration(&id).and_then(|e| e.targets.get(i)).map(|t| t.id.clone());
                    self.begin_link(id, target, out);
                }
            }
            Focus::EngineField(i) => self.change_engine_field(i, true, out),
            Focus::Keyboard(i) => self.keyboard(i, out),
            Focus::SheetAction(i) if self.view.sheet == Some(Layer::Auth) => self.auth_action(i, out),
            _ => {}
        }
    }
    fn accounts_adjust(&mut self, f: Focus, right: bool, out: &mut Vec<Command>) -> bool {
        if let Focus::EngineField(i) = f {
            self.change_engine_field(i, right, out);
            return true;
        }
        let Focus::Setting(row) = f else {
            return false;
        };
        if self.view.page == Page::Accounts {
            let Some(id) = self.view.selected_engine.clone() else {
                return false;
            };
            match row {
                0 => {
                    let enabled = !self.enabled(&id);
                    self.state.engines.entry(id.clone()).or_default().enabled = enabled;
                    out.push(Command::Enable { engine: id, on: enabled });
                    out.push(Command::SaveState);
                    self.rebuild_catalog();
                }
                1 => {
                    if self.enabled(&id) {
                        self.begin_link(id, None, out);
                    }
                }
                2 => {
                    self.view.address.clear();
                    self.open_sheet(Layer::AddHost, Focus::Keyboard(0));
                }
                3 => self.sign_out(id, out),
                4 => self.refresh_libraries(Some(id), out),
                _ => return false,
            }
            return true;
        }
        false
    }
    /// An integration's text field.
    pub(super) fn save_engine_text(&mut self, out: &mut Vec<Command>) {
        let Some(id) = self.view.selected_engine.clone() else {
            return;
        };
        let Some(index) = self.view.edit_field else {
            return;
        };
        let Some(engine) = self.integration(&id) else {
            return;
        };
        let Some(field) = engine.schema.fields().nth(index) else {
            return;
        };
        let value = Value::Text(self.view.address.clone());
        if engine.schema.check(&field.key, &value).is_err() {
            self.view.notice = Some(match field.kind {
                FieldKind::Text { max_chars, .. } => format!("Use at most {max_chars} characters."),
                _ => "The integration changed this field. Open it again.".into(),
            });
            return;
        }
        let key = field.key.clone();
        let saved = self.state.engines.entry(id.clone()).or_default();
        saved.settings.insert(key, value);
        Self::send(out, &id, Request::Settings { values: saved.settings.clone() });
        out.push(Command::SaveState);
        self.close_sheet();
    }
    pub fn auth_remaining(&self) -> Option<u64> {
        self.view.auth.as_ref()?.expires_ms.map(|end| end.saturating_sub(self.now_ms).div_ceil(1000))
    }
    /// Signs an account integration out: the engine forgets the account and what it saved, and the games it listed leave the catalog.
    fn sign_out(&mut self, engine: EngineId, out: &mut Vec<Command>) {
        let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == engine && e.manifest.kind == EngineKind::Account) else {
            return;
        };
        for target in std::mem::take(&mut e.targets) {
            Self::send(out, &engine, Request::Forget { target: target.id.clone() });
            self.listing.forget(&engine, &target.id);
            self.pages.remove(&(engine.clone(), target.id));
        }
        self.view.focus = Focus::Setting(1);
        self.rebuild_catalog();
        out.push(Command::SaveCatalog);
        out.push(Command::SaveState);
    }
    fn change_engine_field(&mut self, index: usize, right: bool, out: &mut Vec<Command>) {
        let Some(id) = self.view.selected_engine.clone() else {
            return;
        };
        let Some(field) = self.integration(&id).and_then(|e| e.schema.fields().nth(index)).cloned() else {
            return;
        };
        if field.kind == FieldKind::Action {
            Self::send(out, &id, Request::Action { key: field.key });
            return;
        }
        let saved = self.state.engines.entry(id.clone()).or_default();
        let current = saved.settings.get(&field.key).cloned().or_else(|| field.kind.default());
        if let FieldKind::Text { .. } = field.kind {
            self.view.address = if let Some(Value::Text(value)) = current { value } else { String::new() };
            self.view.edit_field = Some(index);
            self.open_sheet(Layer::EditText, Focus::Keyboard(0));
            return;
        }
        let next = match (&field.kind, current) {
            (FieldKind::Toggle { .. }, Some(Value::Bool(v))) => Some(Value::Bool(!v)),
            (FieldKind::Choice { options, .. }, Some(Value::Text(v))) if !options.is_empty() => {
                let i = options.iter().position(|o| o.id == v).unwrap_or(0);
                let j = if right { (i + 1).min(options.len() - 1) } else { i.saturating_sub(1) };
                Some(Value::Text(options[j].id.clone()))
            }
            (FieldKind::Number { min, max, step, .. }, Some(Value::Int(v))) => {
                Some(Value::Int(if right { v.saturating_add(*step).min(*max) } else { v.saturating_sub(*step).max(*min) }))
            }
            _ => None,
        };
        if let Some(value) = next {
            saved.settings.insert(field.key, value);
            Self::send(out, &id, Request::Settings { values: saved.settings.clone() });
            out.push(Command::SaveState);
        }
    }
    pub(super) fn begin_link(&mut self, engine: EngineId, target: Option<TargetId>, out: &mut Vec<Command>) {
        let method = self.integration(&engine).and_then(|e| e.descriptor.as_ref()).and_then(|d| d.capabilities.auth.first()).copied();
        self.view.auth = Some(LinkFlow::new(engine.clone(), target.clone(), self.now_ms));
        self.open_sheet(Layer::Auth, Focus::SheetAction(0));
        if !self.enabled(&engine) {
            self.state.engines.entry(engine.clone()).or_default().enabled = true;
            out.push(Command::Enable { engine, on: true });
            out.push(Command::SaveState);
        } else if let Some(method) = method {
            Self::send(out, &engine, Request::Auth { target, method });
        } else {
            self.view.notice = Some("This integration does not need a sign-in.".into());
            self.close_sheet();
        }
    }
    pub(super) fn auth_action(&mut self, action: usize, out: &mut Vec<Command>) {
        let Some(a) = &self.view.auth else {
            return;
        };
        if a.no_pcs {
            let engine = a.engine.clone();
            Self::send(out, &engine, Request::AuthCancel);
            self.view.auth = None;
            self.close_sheet();
            if action == 1 {
                self.view.selected_engine = Some(engine);
                self.view.address.clear();
                self.open_sheet(Layer::AddHost, Focus::Keyboard(0));
            } else {
                Self::send(out, &engine, Request::Targets);
                self.begin_link(engine, None, out);
            }
            return;
        }
        match &a.state {
            Some(AuthState::EnterPin { digits }) if a.pin.len() == usize::from(*digits) => Self::send(out, &a.engine, Request::AuthPin { pin: a.pin.clone() }),
            Some(AuthState::Failed { .. }) => {
                let id = a.engine.clone();
                let target = a.target.clone();
                self.begin_link(id, target, out);
            }
            Some(AuthState::EnterPin { .. }) => {}
            _ => self.go_back(out),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    #[test]
    fn sign_in_starts_one_library_traversal_through_targets() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        let engine = EngineId::new("gfn").unwrap();
        let done = ipc::Event::Auth { state: AuthState::Done { target: TargetId::new("main").unwrap() } };
        let commands = m.step(Input::Engine { engine, message: FromEngine { re: None, event: done } });
        let sent: Vec<_> = commands.iter().filter_map(|c| if let Command::Send { request, .. } = c { Some(request) } else { None }).collect();
        assert!(matches!(sent.as_slice(), [Request::Targets]));
    }
    #[test]
    fn account_approval_retires_only_the_sign_in_warning() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("steam", "Steam", EngineKind::Account)]);
        let steam = EngineId::new("steam").unwrap();
        for code in [ipc::ErrorCode::SignIn, ipc::ErrorCode::Network] {
            engine_says(&mut m, "steam", ipc::Event::Error { code, message: "request refused".into() });
            let target = linked("main");
            engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![target.clone()] });
            assert!(m.integration(&steam).unwrap().error.is_some(), "listing a saved account is not approval");
            engine_says(&mut m, "steam", ipc::Event::Auth { state: AuthState::Done { target: target.id } });
            assert_eq!(m.integration(&steam).unwrap().error.is_some(), code != ipc::ErrorCode::SignIn);
        }
    }
    #[test]
    fn repeated_targets_leave_the_running_library_traversal_alone() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        let engine = EngineId::new("gfn").unwrap();
        let target = ipc::Target { id: TargetId::new("main").unwrap(), name: "nphil".into(), detail: None, link: ipc::Link::Linked, reach: ipc::Reach::Online };
        let deliver = |m: &mut Model, event| {
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event } })
                .into_iter().filter(|c| matches!(c, Command::Send { request: Request::Library { .. }, .. })).count()
        };
        assert_eq!(deliver(&mut m, ipc::Event::Targets { targets: vec![target.clone()] }), 1);
        assert_eq!(deliver(&mut m, ipc::Event::Targets { targets: vec![target.clone()] }), 0);
        assert_eq!(deliver(&mut m, ipc::Event::Library { target: target.id.clone(), entries: vec![], next: None }), 0);
        assert_eq!(deliver(&mut m, ipc::Event::Targets { targets: vec![target] }), 1);
    }
    #[test]
    fn disconnected_engine_targets_stay_offline_until_refreshed() {
        let mut m = Model::new(State::default(), Listing::default(), vec![
            manifest("pc", "PC", EngineKind::Host), manifest("other", "Other", EngineKind::Host),
        ]);
        describe(&mut m, "pc", vec![], &[]);
        describe(&mut m, "other", vec![], &[]);
        let pc = EngineId::new("pc").unwrap();
        let other = EngineId::new("other").unwrap();
        let target = |id: &str| Target {
            id: TargetId::new(id).unwrap(), name: id.into(), detail: None, link: ipc::Link::Linked, reach: ipc::Reach::Online,
        };
        let targets = vec![target("main"), target("second")];
        let entry = ipc::LibraryEntry {
            id: lumagame_core::ids::ProviderGameId::new("1").unwrap(), title: "Game".into(), role: ipc::Role::Game,
            external: vec![], store: None, store_account: None, art: ipc::Art::default(), readiness: Readiness::Ready, resumable: false, activity: Default::default(),
        };
        let deliver = |m: &mut Model, engine: &EngineId, event| {
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event } })
        };
        deliver(&mut m, &pc, ipc::Event::Targets { targets: targets.clone() });
        deliver(&mut m, &pc, ipc::Event::Library { target: targets[0].id.clone(), entries: vec![entry.clone()], next: None });
        deliver(&mut m, &other, ipc::Event::Targets { targets: vec![target("main")] });
        deliver(&mut m, &other, ipc::Event::Library { target: targets[0].id.clone(), entries: vec![entry.clone()], next: None });
        let source: SourceKey = "pc|main|1".parse().unwrap();
        let other_source: SourceKey = "other|main|1".parse().unwrap();
        let game = m.games().iter().find(|g| g.source(&source).is_some()).unwrap().id;
        m.state.profile_mut().user.started(game, &source, true);
        let saved = m.state.profile().user.clone();
        let games = m.games().to_vec();
        let descriptor = m.integration(&pc).unwrap().descriptor.clone().unwrap();
        assert_eq!(m.readiness(&source), Readiness::Ready);
        assert!(matches!(m.resolution(game), lumagame_core::launch::Resolution::Launch { .. }));

        m.step(Input::EngineGone(pc.clone()));
        assert!(m.integration(&pc).unwrap().targets.iter().all(|t| t.reach == ipc::Reach::Offline && t.link == ipc::Link::Linked));
        assert_eq!(m.readiness(&source), Readiness::Offline);
        assert_eq!(m.readiness(&other_source), Readiness::Ready);
        assert_eq!(m.games(), games);
        assert_eq!(m.state.profile().user, saved);
        m.open_game(game, None);
        m.rebuild_navigation();
        assert!(!sent(&key(&mut m, Key::Ok)).iter().any(|r| matches!(r, Request::Launch { .. })));
        assert_eq!(m.view.sheet, Some(Layer::Chooser));
        key(&mut m, Key::Ok);
        assert!(!sent(&key(&mut m, Key::Ok)).iter().any(|r| matches!(r, Request::Launch { .. })));

        let mut expired = targets.clone();
        expired[0].link = ipc::Link::Expired;
        deliver(&mut m, &pc, ipc::Event::Targets { targets: expired });
        m.step(Input::EngineGone(pc.clone()));
        assert_eq!(m.target(&source).unwrap().link, ipc::Link::Expired);
        assert_eq!(m.readiness(&source), Readiness::Offline);
        m.open_game(game, None);
        m.rebuild_navigation();
        assert!(!sent(&key(&mut m, Key::Ok)).iter().any(|r| matches!(r, Request::Launch { .. })));
        assert_eq!(m.view.sheet, Some(Layer::Chooser));
        key(&mut m, Key::Ok);
        assert!(!sent(&key(&mut m, Key::Ok)).iter().any(|r| matches!(r, Request::Launch { .. })));

        deliver(&mut m, &pc, ipc::Event::Ready { descriptor, schema: SettingsSchema::default() });
        assert_eq!(m.readiness(&source), Readiness::Offline);
        deliver(&mut m, &pc, ipc::Event::Targets { targets: targets.clone() });
        deliver(&mut m, &pc, ipc::Event::Library { target: targets[0].id.clone(), entries: vec![entry], next: None });
        assert_eq!(m.readiness(&source), Readiness::Ready);
        assert_eq!(m.state.profile().user, saved);
        m.open_game(game, None);
        m.rebuild_navigation();
        assert!(key(&mut m, Key::Ok).iter().any(|c| matches!(c, Command::Send { engine, request: Request::Launch { .. } } if engine == &pc)));
    }
    #[test]
    fn integration_schema_updates_keep_focus_on_the_same_field_or_a_safe_control() {
        let id = EngineId::new("native").unwrap();
        let mut m = Model::new(State::default(), Listing::default(), vec![
            manifest("native", "This TV", EngineKind::Native), manifest("other", "Other", EngineKind::Native),
        ]);
        describe(&mut m, "native", vec![], &[]);
        describe(&mut m, "other", vec![], &[]);
        let schema = |keys: &[&str]| SettingsSchema { sections: vec![lumagame_core::settings::Section {
            title: "Apps".into(),
            fields: keys.iter().map(|key| lumagame_core::settings::Field {
                key: (*key).into(), label: (*key).into(), help: None, kind: FieldKind::Toggle { default: false },
            }).collect(),
        }] };
        m.integrations[0].schema = schema(&["first", "proof", "third"]);
        m.view.page = Page::Accounts;
        m.view.selected_engine = Some(id.clone());
        m.view.focus = Focus::EngineField(1);
        m.rebuild_navigation();
        key(&mut m, Key::Ok);
        key(&mut m, Key::Ok);
        assert_eq!(m.state.engines[&id].settings.get("proof"), Some(&Value::Bool(false)));
        assert!(!m.state.engines[&id].settings.contains_key("first"));
        let descriptor = m.integrations[0].descriptor.clone().unwrap();
        let other = EngineId::new("other").unwrap();
        let other_descriptor = m.integration(&other).unwrap().descriptor.clone().unwrap();
        m.step(Input::Engine { engine: other, message: FromEngine { re: None,
            event: ipc::Event::Ready { descriptor: other_descriptor, schema: schema(&["unrelated"]) } } });
        assert_eq!(m.view.focus, Focus::EngineField(1), "a background integration cannot move focus");
        m.step(Input::Key { key: Key::Ok, down: true });
        m.step(Input::Engine { engine: id.clone(), message: FromEngine { re: None,
            event: ipc::Event::Ready { descriptor: descriptor.clone(), schema: schema(&["proof", "first", "third"]) } } });
        assert_eq!(m.view.focus, Focus::EngineField(0));
        m.step(Input::Key { key: Key::Ok, down: false });
        assert_eq!(m.state.engines[&id].settings.get("proof"), Some(&Value::Bool(false)), "a held press is cancelled across a reorder");
        key(&mut m, Key::Ok);
        assert_eq!(m.state.engines[&id].settings.get("proof"), Some(&Value::Bool(true)));
        m.step(Input::Engine { engine: id.clone(), message: FromEngine { re: None,
            event: ipc::Event::Ready { descriptor, schema: schema(&["first", "third"]) } } });
        assert_eq!(m.view.focus, Focus::Nav(3), "a missing or off-page app must not focus another app or switch");
        assert!(!m.state.engines[&id].settings.contains_key("first"));
    }

    #[test]
    fn this_tv_selection_and_discovery_refresh_the_visible_library() {
        use crate::game::native::{self, Native};
        use crate::lifecycle::{FromApp, InstalledApp, ToApp};
        use lumagame_core::engine::Engine;
        use lumagame_core::ipc::{Display, Hello, ToEngine, PROTOCOL};

        fn exchange(model: &mut Model, engine: &mut Native, commands: Vec<Command>, seq: &mut u64) {
            let mut commands = std::collections::VecDeque::from(commands);
            loop {
                while let Some(command) = commands.pop_front() {
                    if let Command::Send { engine: id, request } = command {
                        assert_eq!(id, native::id());
                        *seq += 1;
                        engine.send(ToEngine { seq: *seq, request });
                    }
                }
                let Some(message) = engine.poll() else { break };
                commands.extend(model.step(Input::Engine { engine: native::id(), message }));
            }
        }

        let id = native::id();
        let mut model = Model::new(State::default(), Listing::default(), vec![native::manifest()]);
        model.state.engines.entry(id.clone()).or_default().enabled = true;
        let mut engine = Native::new(true);
        let mut seq = 1;
        engine.send(ToEngine { seq, request: Request::Hello(Hello {
            protocol: PROTOCOL, app_version: "test".into(), data_dir: String::new(), cache_dir: String::new(),
            feeder: None, locale: "en-US".into(), display: Display { width: 1920, height: 1080, refresh_hz: 60, hdr10: false, codecs: vec![] },
            settings: Values::new(),
        }) });
        exchange(&mut model, &mut engine, vec![], &mut seq);
        let FromApp::Apps { seq: listing } = engine.command().unwrap() else { panic!("app discovery") };
        let app = InstalledApp { id: "com.example.proof".into(), title: "Native proof".into(), icon: None };
        engine.lifecycle(&ToApp::Apps { seq: listing, apps: vec![app.clone()], success: true, message: String::new() });
        exchange(&mut model, &mut engine, vec![], &mut seq);
        assert!(model.games().is_empty());

        model.view.selected_engine = Some(id.clone());
        let mut commands = Vec::new();
        model.change_engine_field(0, true, &mut commands);
        assert!(commands.contains(&Command::SaveState));
        assert!(model.state.engines[&id].settings.values().any(|v| *v == Value::Bool(true)));
        exchange(&mut model, &mut engine, commands, &mut seq);
        assert_eq!(model.games().len(), 1);
        assert_eq!(model.games()[0].title, "Native proof");

        engine.send(ToEngine { seq: { seq += 1; seq }, request: Request::Action { key: "refresh-apps".into() } });
        let FromApp::Apps { seq: listing } = engine.command().unwrap() else { panic!("app refresh") };
        engine.lifecycle(&ToApp::Apps { seq: listing, apps: vec![], success: true, message: String::new() });
        exchange(&mut model, &mut engine, vec![], &mut seq);
        assert!(model.games().is_empty());

        engine.send(ToEngine { seq: { seq += 1; seq }, request: Request::Action { key: "refresh-apps".into() } });
        let FromApp::Apps { seq: listing } = engine.command().unwrap() else { panic!("app refresh") };
        engine.lifecycle(&ToApp::Apps { seq: listing, apps: vec![app], success: true, message: String::new() });
        exchange(&mut model, &mut engine, vec![], &mut seq);
        assert_eq!(model.games().len(), 1, "the saved selection returns with the installed app");

        let mut commands = Vec::new();
        model.change_engine_field(0, false, &mut commands);
        exchange(&mut model, &mut engine, commands, &mut seq);
        assert!(model.games().is_empty());
    }

    #[test]
    fn an_account_is_asked_again_after_a_stream_and_signing_out_clears_what_it_listed() {
        use lumagame_core::activity::Activity;
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[]);
        let steam = EngineId::new("steam").unwrap();
        let account = TargetId::new("76561198000000000").unwrap();
        engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked("76561198000000000")] });
        let records = vec![entry("10", "10", ipc::Role::Record, &["steam:10"], Activity { last_played: Some(5), ..Activity::default() })];
        engine_says(&mut m, "steam", ipc::Event::Library { target: account.clone(), entries: records.clone(), next: None });
        let enabled = BTreeSet::from([steam.clone()]);
        assert_eq!(m.listing.enabled(&enabled).len(), 1);

        let mut out = Vec::new();
        m.refresh_accounts(false, &mut out);
        assert!(sent(&out).is_empty(), "an account listed a moment ago is not asked again");
        m.step(Input::Tick { now_ms: 300_000, unix_s: 0 });
        m.refresh_accounts(false, &mut out);
        assert!(matches!(sent(&out)[..], [Request::Library { cursor: None, .. }]), "returning from a stream asks the signed-in account again");
        let mut again = Vec::new();
        m.refresh_accounts(true, &mut again);
        assert!(sent(&again).is_empty(), "a traversal under way is left to finish, even when forced");
        engine_says(&mut m, "steam", ipc::Event::Library { target: account.clone(), entries: records, next: None });
        let mut forced = Vec::new();
        m.refresh_accounts(true, &mut forced);
        assert!(matches!(sent(&forced)[..], [Request::Action { .. }, Request::Library { cursor: None, .. }]), "a forced refresh first tells the engine to skip its saved copy");

        let mut out = Vec::new();
        m.sign_out(steam.clone(), &mut out);
        assert!(matches!(sent(&out)[..], [Request::Forget { .. }]));
        assert!(m.integration(&steam).unwrap().targets.is_empty() && !m.integration(&steam).unwrap().can_sign_out());
        assert!(m.listing.enabled(&enabled).is_empty(), "what the account listed leaves the catalog");
        let mut after = Vec::new();
        m.refresh_accounts(true, &mut after);
        assert!(sent(&after).is_empty(), "a signed-out account is not asked");
    }
    #[test]
    fn a_qr_sign_in_lasts_as_long_as_the_whole_attempt_however_often_the_code_rotates() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[]);
        let qr = |expires_s| ipc::Event::Auth { state: AuthState::Qr { url: "https://s.team/q/1/2".into(), expires_s } };
        engine_says(&mut m, "steam", qr(300));
        assert_eq!(m.view.auth.as_ref().and_then(|a| a.expires_ms), Some(300_000));
        m.step(Input::Tick { now_ms: 100_000, unix_s: 0 });
        engine_says(&mut m, "steam", qr(200));
        assert_eq!(m.view.auth.as_ref().and_then(|a| a.expires_ms), Some(300_000), "a rotated code keeps the same end");
        m.step(Input::Tick { now_ms: 299_999, unix_s: 0 });
        assert!(matches!(m.view.auth.as_ref().and_then(|a| a.state.as_ref()), Some(AuthState::Qr { .. })));
        m.step(Input::Tick { now_ms: 300_000, unix_s: 0 });
        assert_eq!(m.auth_remaining(), Some(0));
        assert!(matches!(m.view.auth.as_ref().and_then(|a| a.state.as_ref()), Some(AuthState::Qr { .. })));
        engine_says(&mut m, "steam", ipc::Event::Auth { state: AuthState::Done { target: TargetId::new("main").unwrap() } });
        assert!(m.view.auth.is_none(), "the final Steam reply closes the sheet without offering a replacement code");
    }
    #[test]
    fn pairing_with_no_pc_found_offers_an_address_or_another_search() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gamestream", "Sunshine", EngineKind::Host)]);
        describe(&mut m, "gamestream", vec![lumagame_core::engine::AuthMethod::ShowPin], &[Feature::ManualHost]);
        let id = EngineId::new("gamestream").unwrap();
        let mut out = Vec::new();
        m.begin_link(id.clone(), None, &mut out);
        m.step(Input::Tick { now_ms: PC_SEARCH_MS - 1, unix_s: 0 });
        assert!(!m.view.auth.as_ref().unwrap().no_pcs);
        m.step(Input::Tick { now_ms: PC_SEARCH_MS, unix_s: 0 });
        assert!(m.view.auth.as_ref().unwrap().no_pcs);
        assert!(m.navigation.iter().any(|n| n.id == Focus::SheetAction(1)));

        m.view.focus = Focus::SheetAction(0);
        let retry = key(&mut m, Key::Ok);
        assert!(matches!(sent(&retry).as_slice(), [Request::AuthCancel, Request::Targets, Request::Auth { target: None, .. }]));
        let flow = m.view.auth.as_ref().unwrap();
        assert!(!flow.no_pcs && flow.since_ms == PC_SEARCH_MS, "a retry searches for the full time again");

        m.step(Input::Tick { now_ms: 2 * PC_SEARCH_MS, unix_s: 0 });
        m.view.focus = Focus::SheetAction(1);
        let manual = key(&mut m, Key::Ok);
        assert!(matches!(sent(&manual).as_slice(), [Request::AuthCancel]));
        assert_eq!((m.view.sheet, m.view.focus), (Some(Layer::AddHost), Focus::Keyboard(0)));
        assert_eq!(m.view.selected_engine, Some(id.clone()));

        let cancelled = AuthState::Failed { reason: ipc::AuthFail::Cancelled, message: "Link cancelled.".into() };
        m.step(Input::Engine { engine: id, message: FromEngine { re: None, event: ipc::Event::Auth { state: cancelled } } });
        assert_eq!(m.view.sheet, Some(Layer::AddHost), "the answer to the cancel leaves the address entry open");
    }

}
