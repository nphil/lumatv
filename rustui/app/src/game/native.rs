//! TV games use the feeder's LS2 handle; Running means observed in the foreground, not merely accepted by launch.
use crate::lifecycle::{FromApp, InstalledApp, ToApp};
use lumagame_core::engine::{Capabilities, Descriptor, Engine, EngineKind, Feature, Manifest, Presentation, TargetKind};
use lumagame_core::ids::{EngineId, ExternalId, ProviderGameId, SessionId, TargetId};
use lumagame_core::ipc::{Art, EndReason, ErrorCode, Event, FailReason, FromEngine, LibraryEntry, Link, PROTOCOL, Reach, Readiness, Request, Role, SessionState, Target, ToEngine};
use lumagame_core::settings::{Field, FieldKind, Section, SettingsSchema, Value, Values};
use std::collections::{BTreeSet, VecDeque};

const PAGE: usize = 120;

pub fn id() -> EngineId { EngineId::new("native").expect("native engine id") }
fn target() -> TargetId { TargetId::new("tv").expect("TV target id") }

pub fn manifest() -> Manifest {
    Manifest { id: id(), name: "This TV".into(), summary: "Choose installed TV apps to include as games.".into(), kind: EngineKind::Native,
        protocol: PROTOCOL, exec: None, args: Vec::new(), license: "MIT".into(), source: None }
}

fn setting(id: &str) -> String {
    let hash = id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    format!("game-{hash:016x}")
}

struct Active {
    session: SessionId,
    app_id: String,
    request: u64,
    launch: u64,
    accepted: bool,
    running: bool,
}

pub struct Native {
    available: bool,
    apps: Vec<InstalledApp>,
    settings: Values,
    page: usize,
    next: u64,
    listing: Option<u64>,
    foreground: String,
    active: Option<Active>,
    events: VecDeque<FromEngine>,
    commands: VecDeque<FromApp>,
}

impl Native {
    pub fn new(available: bool) -> Self {
        Self { available, apps: Vec::new(), settings: Values::new(), page: 0, next: 1, listing: None, foreground: String::new(), active: None, events: VecDeque::new(), commands: VecDeque::new() }
    }

    pub fn command(&mut self) -> Option<FromApp> { self.commands.pop_front() }

    fn seq(&mut self) -> u64 { let seq = self.next; self.next += 1; seq }
    fn answer(&mut self, seq: u64, event: Event) { self.events.push_back(FromEngine::answer(seq, event)); }
    fn error(&mut self, seq: u64, code: ErrorCode, message: impl Into<String>) { self.answer(seq, Event::Error { code, message: message.into() }); }

    fn ready(&mut self, re: Option<u64>) {
        let start = self.page * PAGE;
        let mut fields: Vec<Field> = self.apps.iter().skip(start).take(PAGE).map(|app| Field {
            key: setting(&app.id), label: app.title.chars().take(128).collect(), help: None, kind: FieldKind::Toggle { default: false },
        }).collect();
        if self.page > 0 { fields.push(Field { key: "previous-apps".into(), label: "Previous apps".into(), help: None, kind: FieldKind::Action }); }
        if start + PAGE < self.apps.len() { fields.push(Field { key: "next-apps".into(), label: "More apps".into(), help: None, kind: FieldKind::Action }); }
        fields.push(Field { key: "refresh-apps".into(), label: "Refresh installed apps".into(), help: None, kind: FieldKind::Action });
        let schema = SettingsSchema { sections: vec![Section { title: "Include as games".into(), fields }] };
        let descriptor = Descriptor { id: id(), version: env!("CARGO_PKG_VERSION").into(), capabilities: Capabilities {
            kind: EngineKind::Native, targets: TargetKind::Device, auth: Vec::new(), presentation: Presentation::App,
            features: BTreeSet::from([Feature::Readiness]), video: None,
        } };
        self.events.push_back(FromEngine { re, event: Event::Ready { descriptor, schema } });
    }

    fn list_apps(&mut self) {
        if self.available {
            let seq = self.seq();
            self.listing = Some(seq);
            self.commands.push_back(FromApp::Apps { seq });
        }
    }

    fn library(&self) -> Vec<LibraryEntry> {
        self.apps.iter().filter(|app| self.settings.get(&setting(&app.id)) == Some(&Value::Bool(true))).filter_map(|app| {
            Some(LibraryEntry { id: ProviderGameId::new(&app.id).ok()?, title: app.title.clone(), role: Role::Game,
                external: vec![ExternalId::new(ExternalId::WEBOS, &app.id).ok()?], store: None, store_account: None,
                art: Art { icon: app.icon.clone(), ..Art::default() }, readiness: Readiness::Ready, resumable: false, activity: Default::default() })
        }).collect()
    }

    fn tell_library(&mut self, seq: u64) { self.answer(seq, Event::Library { target: target(), entries: self.library(), next: None }); }

    fn targets(&self) -> Event {
        Event::Targets { targets: vec![Target { id: target(), name: "This TV".into(), detail: None, avatar: None,
            link: Link::Linked, reach: if self.available { Reach::Online } else { Reach::Offline } }] }
    }

    fn finish(&mut self, state: SessionState) {
        if let Some(active) = self.active.take() {
            self.commands.retain(|command| !matches!(command, FromApp::Launch { seq, .. } if *seq == active.launch));
            self.answer(active.request, Event::Session { session: active.session, state });
        }
    }

    fn maybe_running(&mut self) {
        let Some(active) = &mut self.active else { return };
        if active.accepted && !active.running && self.foreground == active.app_id {
            active.running = true;
            self.events.push_back(FromEngine::answer(active.request, Event::Session { session: active.session, state: SessionState::Running { stream: None } }));
        }
    }

    pub fn lifecycle(&mut self, message: &ToApp) {
        match message {
            ToApp::Apps { seq, apps, success, message } if self.listing == Some(*seq) => {
                self.listing = None;
                if *success {
                    self.apps = apps.iter().filter(|app| app.id != super::store::APP_ID && !app.title.is_empty() && ExternalId::new(ExternalId::WEBOS, &app.id).is_ok()).cloned().collect();
                    self.apps.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
                    self.apps.dedup_by(|a, b| a.id == b.id);
                    // Freeze this ordering until discovery runs again, so toggling never moves the focused row.
                    let mut left = 0;
                    let mut right = self.apps.len();
                    while left < right {
                        if self.settings.get(&setting(&self.apps[left].id)) == Some(&Value::Bool(true)) {
                            left += 1;
                        } else {
                            right -= 1;
                            self.apps.swap(left, right);
                        }
                    }
                    self.apps[..left].sort_unstable_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
                    self.apps[left..].sort_unstable_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
                    self.page = self.page.min(self.apps.len().saturating_sub(1) / PAGE);
                    self.ready(None);
                } else { self.error(*seq, ErrorCode::Internal, message); }
            }
            ToApp::Launched { seq, app_id, success, message } if self.active.as_ref().is_some_and(|a| a.launch == *seq && a.app_id == *app_id) => {
                if *success {
                    self.active.as_mut().unwrap().accepted = true;
                    self.maybe_running();
                } else {
                    self.finish(SessionState::Failed { reason: FailReason::Engine, message: message.clone() });
                }
            }
            ToApp::Foreground { app_id } => {
                self.foreground.clone_from(app_id);
                if self.active.as_ref().is_some_and(|a| a.running && a.app_id != *app_id) {
                    self.finish(SessionState::Ended { reason: EndReason::Remote });
                } else { self.maybe_running(); }
            }
            _ => {}
        }
    }

    pub fn channel_failed(&mut self, message: String) {
        self.available = false;
        self.commands.clear();
        self.finish(SessionState::Failed { reason: FailReason::Engine, message });
    }
}

impl Engine for Native {
    fn send(&mut self, ToEngine { seq, request }: ToEngine) {
        match request {
            Request::Hello(hello) => {
                if hello.protocol != PROTOCOL { self.error(seq, ErrorCode::Protocol, "Unsupported engine protocol."); return; }
                self.settings = hello.settings;
                self.ready(Some(seq));
                self.list_apps();
            }
            Request::Settings { values } => {
                self.settings = values;
                self.answer(seq, self.targets());
            }
            Request::Targets => self.answer(seq, self.targets()),
            Request::Library { target: asked, .. } if asked == target() => self.tell_library(seq),
            Request::Check { target: asked, game } => {
                let present = asked == target() && self.library().iter().any(|entry| entry.id == game);
                self.answer(seq, Event::Readiness { target: asked, game, readiness: if !self.available { Readiness::Offline } else if present { Readiness::Ready } else { Readiness::Install } });
            }
            Request::Launch { session, target: asked, game } => {
                let failure = if self.active.is_some() { Some((FailReason::Busy, "A TV game is already tracked.")) }
                    else if !self.available { Some((FailReason::Engine, "The TV application service is unavailable.")) }
                    else if asked != target() || !self.library().iter().any(|entry| entry.id == game) { Some((FailReason::NotInstalled, "That TV game is no longer selected or installed.")) }
                    else { None };
                if let Some((reason, message)) = failure {
                    self.answer(seq, Event::Session { session, state: SessionState::Failed { reason, message: message.into() } });
                    return;
                }
                let launch = self.seq();
                self.answer(seq, Event::Session { session, state: SessionState::Checking });
                self.answer(seq, Event::Session { session, state: SessionState::Starting });
                self.commands.push_back(FromApp::Launch { seq: launch, app_id: game.as_str().into(), parameters: serde_json::json!({}) });
                self.active = Some(Active { session, app_id: game.as_str().into(), request: seq, launch, accepted: false, running: false });
            }
            Request::Stop { session, .. } if self.active.as_ref().is_some_and(|a| a.session == session) => self.finish(SessionState::Ended { reason: EndReason::Stopped }),
            Request::Action { key } if key == "refresh-apps" => self.list_apps(),
            Request::Action { key } if key == "next-apps" || key == "previous-apps" => {
                self.page = if key == "previous-apps" { self.page.saturating_sub(1) } else { (self.page + 1).min(self.apps.len().saturating_sub(1) / PAGE) };
                self.ready(Some(seq));
            }
            _ => self.error(seq, ErrorCode::Unsupported, "This TV engine does not support that request."),
        }
    }

    fn poll(&mut self) -> Option<FromEngine> { self.events.pop_front() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_places_saved_apps_first_without_changing_selection_or_reordering_on_toggle() {
        let mut engine = Native::new(true);
        let chosen = setting("com.example.app082");
        engine.settings.insert(chosen.clone(), Value::Bool(true));
        engine.settings.insert(setting("com.example.app100"), Value::Bool(true));
        let saved = engine.settings.clone();
        let apps: Vec<_> = (0..143).map(|i| InstalledApp {
            id: format!("com.example.app{i:03}"), title: format!("App {i:03}"), icon: None,
        }).collect();
        engine.list_apps();
        let FromApp::Apps { seq } = engine.command().unwrap() else { panic!() };
        engine.lifecycle(&ToApp::Apps { seq, apps, success: true, message: String::new() });
        assert_eq!(engine.settings, saved);
        assert_eq!(engine.apps[0].id, "com.example.app082");
        assert_eq!(engine.apps[1].id, "com.example.app100");
        assert_eq!(engine.apps[2].id, "com.example.app000");
        assert_eq!(engine.apps[83].id, "com.example.app081");
        assert_eq!(engine.apps[84].id, "com.example.app083");
        let before: Vec<_> = engine.apps.iter().map(|app| app.id.clone()).collect();
        engine.send(ToEngine { seq: 4, request: Request::Settings { values: Values::new() } });
        assert_eq!(engine.apps.iter().map(|app| &app.id).collect::<Vec<_>>(), before.iter().collect::<Vec<_>>());
        assert!(engine.library().is_empty());
    }

    #[test]
    fn only_selected_apps_launch_and_only_foreground_starts_playtime() {
        let mut engine = Native::new(true);
        engine.list_apps();
        let FromApp::Apps { seq } = engine.command().unwrap() else { panic!() };
        engine.lifecycle(&ToApp::Apps { seq, apps: vec![InstalledApp { id: "com.game".into(), title: "Game".into(), icon: None }], success: true, message: String::new() });
        assert!(engine.library().is_empty());
        engine.send(ToEngine { seq: 2, request: Request::Settings { values: Values::from([(setting("com.game"), Value::Bool(true))]) } });
        assert_eq!(engine.library().len(), 1);
        engine.events.clear();
        engine.send(ToEngine { seq: 3, request: Request::Launch { session: SessionId(1), target: target(), game: ProviderGameId::new("com.game").unwrap() } });
        let FromApp::Launch { seq, .. } = engine.command().unwrap() else { panic!() };
        engine.lifecycle(&ToApp::Launched { seq, app_id: "com.game".into(), success: true, message: String::new() });
        assert!(!engine.active.as_ref().unwrap().running);
        engine.lifecycle(&ToApp::Foreground { app_id: "com.game".into() });
        assert!(engine.active.as_ref().unwrap().running);
        engine.lifecycle(&ToApp::Foreground { app_id: "org.nphil.lumagame".into() });
        let states: Vec<_> = engine.events.into_iter().filter_map(|m| match m.event { Event::Session { state, .. } => Some(state), _ => None }).collect();
        assert_eq!(states, [SessionState::Checking, SessionState::Starting, SessionState::Running { stream: None }, SessionState::Ended { reason: EndReason::Remote }]);
    }

    #[test]
    fn a_foreground_event_before_the_launch_reply_is_not_lost() {
        let mut engine = Native::new(true);
        engine.active = Some(Active { session: SessionId(1), app_id: "game".into(), request: 2, launch: 3, accepted: false, running: false });
        engine.lifecycle(&ToApp::Foreground { app_id: "game".into() });
        assert!(engine.events.is_empty());
        engine.lifecycle(&ToApp::Launched { seq: 3, app_id: "game".into(), success: true, message: String::new() });
        assert!(matches!(engine.poll().unwrap().event, Event::Session { state: SessionState::Running { .. }, .. }));
    }

    #[test]
    fn cancelling_before_the_lifecycle_write_does_not_launch_the_app() {
        let mut engine = Native::new(true);
        engine.active = Some(Active { session: SessionId(1), app_id: "game".into(), request: 2, launch: 3, accepted: false, running: false });
        engine.commands.push_back(FromApp::Launch { seq: 3, app_id: "game".into(), parameters: serde_json::json!({}) });
        engine.send(ToEngine { seq: 4, request: Request::Stop { session: SessionId(1), end_game: false } });
        assert!(engine.command().is_none());
        assert!(matches!(engine.poll().unwrap().event, Event::Session { state: SessionState::Ended { .. }, .. }));
    }
}
