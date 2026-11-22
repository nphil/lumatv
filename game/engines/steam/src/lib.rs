//! LumaGame's Steam account engine: a program the app starts and talks to in [`lumagame_core::ipc`] on its standard streams.
//!
//! It plays nothing. It signs the owner in by QR code and reports, for each game the account owns, when it was last played, for how
//! long and what kind of game it is; the app puts that on the games it already lists from other engines. Steam's own Favorites and
//! collections live in the Steam client and are out of reach.
//!
//! [`Adapter`] is the whole engine without a process around it: it takes a request, answers with events, and says when it next has to
//! run. The network and the clock are handed in, so every path is tried against fakes; [`runtime`] owns the pipes and the sleeping.
//!
//! Nothing that identifies the owner or opens the account is ever logged or sent to the app: no token, no URL that carries one, no
//! login name. The one address that is sent is the QR code's, which is meant to be shown.

mod auth;
#[cfg(test)]
mod fake;
mod files;
pub mod http;
mod library;
pub mod runtime;
mod store;

use auth::{Account, Attempt};
use http::Http;
use library::Snapshot;
use lumagame_core::engine::{AuthMethod, Capabilities, Descriptor, EngineKind, Presentation, TargetKind};
use lumagame_core::ids::{EngineId, TargetId};
use lumagame_core::ipc::{ErrorCode, Event, FromEngine, Hello, Link, PROTOCOL, Reach, Request, Target};
use lumagame_core::settings::{Field, FieldKind, Section, SettingsSchema};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use store::Store;

/// Time as the engine reads it: monotonic milliseconds for waiting, wall-clock seconds for what is saved and for token lifetimes.
pub trait Clock {
    fn mono_ms(&self) -> u64;
    fn unix_s(&self) -> u64;
}

pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn mono_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn unix_s(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing heard yet: the first request must be a hello.
    Awaiting,
    Ready,
    /// The engine gave up on the conversation and the runtime ends it.
    Finished,
}

pub struct Adapter<H, C> {
    pub(crate) http: H,
    pub(crate) clock: C,
    log: Box<dyn FnMut(&str)>,
    phase: Phase,
    /// What must survive: the sign-in. Set by hello.
    pub(crate) data: PathBuf,
    /// What can be fetched again: the library and the store's answers. Set by hello.
    pub(crate) cache: PathBuf,
    pub(crate) account: Option<Account>,
    pub(crate) attempt: Option<Attempt>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) store: Option<Store>,
    /// The owner asked for a refresh: the next start of a library round goes to Steam whatever the wait.
    pub(crate) refresh_wanted: bool,
    /// The events of the call in progress.
    out: Vec<FromEngine>,
    /// The account's state changed on its own, and the app is told once the reply that caused it is out.
    targets_changed: bool,
}

impl<H: Http, C: Clock> Adapter<H, C> {
    pub fn new(http: H, clock: C, log: impl FnMut(&str) + 'static) -> Self {
        Self {
            http,
            clock,
            log: Box::new(log),
            phase: Phase::Awaiting,
            data: PathBuf::new(),
            cache: PathBuf::new(),
            account: None,
            attempt: None,
            snapshot: None,
            store: None,
            refresh_wanted: false,
            out: Vec::new(),
            targets_changed: false,
        }
    }

    /// Whether the engine has nothing more to say and the runtime should end it.
    pub fn finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    /// One request, and what it makes the engine say, in order.
    pub fn handle(&mut self, seq: u64, request: Request) -> Vec<FromEngine> {
        match (self.phase, request) {
            (Phase::Awaiting, Request::Hello(hello)) => self.hello(seq, &hello),
            (Phase::Awaiting, _) => {
                self.phase = Phase::Finished;
                self.error(seq, ErrorCode::Protocol, "The first request must be a compatible hello");
            }
            (Phase::Ready, Request::Hello(_)) => self.error(seq, ErrorCode::BadRequest, "Hello was already received"),
            (Phase::Ready, request) => self.dispatch(seq, request),
            (Phase::Finished, _) => {}
        }
        self.flush()
    }

    /// What is due by now: the next check of a sign-in in progress, or its end.
    pub fn tick(&mut self) -> Vec<FromEngine> {
        if self.phase == Phase::Ready {
            self.poll_due();
        }
        self.flush()
    }

    /// Milliseconds from now until [`Adapter::tick`] has something to do, or `None` when it has nothing to wait for.
    pub fn next_wake(&self) -> Option<u64> {
        if self.phase == Phase::Ready { self.auth_wake() } else { None }
    }

    fn hello(&mut self, seq: u64, hello: &Hello) {
        if hello.protocol != PROTOCOL {
            self.phase = Phase::Finished;
            return self.error(seq, ErrorCode::Protocol, "The first request must be a compatible hello");
        }
        let (data, cache) = (PathBuf::from(&hello.data_dir), PathBuf::from(&hello.cache_dir));
        if !data.is_absolute() || !cache.is_absolute() {
            self.phase = Phase::Finished;
            return self.error(seq, ErrorCode::BadRequest, "The data and cache directories must be absolute paths");
        }
        (self.data, self.cache) = (data, cache);
        self.account = Account::load(&self.data, self.clock.unix_s());
        self.note(if self.account.is_some() { "ready, signed in" } else { "ready, signed out" });
        self.phase = Phase::Ready;
        self.answer(seq, Event::Ready { descriptor: descriptor(), schema: schema() });
    }

    fn dispatch(&mut self, seq: u64, request: Request) {
        match request {
            // The app sends a refresh just before it asks for the library, so the request is kept for the round that follows.
            Request::Action { key } if key == "refresh" => self.refresh_wanted = true,
            // Nothing else here is configurable, and an action field is one this engine never offers.
            Request::Settings { .. } | Request::Action { .. } => {}
            Request::Targets => self.answer(seq, Event::Targets { targets: self.targets() }),
            Request::Auth { method: AuthMethod::Qr, .. } => self.begin_auth(seq),
            Request::AuthCancel => self.cancel_auth(seq),
            Request::Forget { target } => self.forget(seq, &target),
            Request::Library { target, cursor } => self.library(seq, target, cursor.as_deref()),
            _ => self.error(seq, ErrorCode::Unsupported, "This operation is not supported by Steam"),
        }
    }

    /// Unlinks the account and deletes everything kept for it. Safe to repeat, and safe when signed out.
    fn forget(&mut self, seq: u64, target: &TargetId) {
        if self.account.as_ref().is_some_and(|account| account.steamid != target.as_str()) {
            return self.error(seq, ErrorCode::BadRequest, "That is not the signed-in Steam account");
        }
        // The sign-in goes first: if it cannot be deleted the account stays, so that memory and disk agree.
        if let Err(error) = files::remove(&self.data, auth::TOKEN_FILE) {
            self.note(&format!("forget: the saved sign-in could not be deleted ({:?})", error.kind()));
            return self.error(seq, ErrorCode::Internal, "The saved Steam sign-in could not be removed");
        }
        for name in [library::CACHE_FILE, store::CACHE_FILE] {
            if let Err(error) = files::remove(&self.cache, name) {
                self.note(&format!("forget: a saved copy could not be deleted ({:?})", error.kind()));
            }
        }
        self.account = None;
        self.snapshot = None;
        self.store = None;
        self.answer(seq, Event::Targets { targets: Vec::new() });
    }

    /// The signed-in account, or none.
    fn targets(&self) -> Vec<Target> {
        self.account
            .iter()
            .map(|account| Target {
                id: account.target_id(),
                name: account.name(),
                detail: None,
                link: if account.expired { Link::Expired } else { Link::Linked },
                reach: Reach::Online,
            })
            .collect()
    }

    /// Ends the call: tells the app of a change the call made on its own, and hands back everything said.
    fn flush(&mut self) -> Vec<FromEngine> {
        if std::mem::take(&mut self.targets_changed) {
            let targets = self.targets();
            self.told(Event::Targets { targets });
        }
        std::mem::take(&mut self.out)
    }

    pub(crate) fn answer(&mut self, seq: u64, event: Event) {
        self.out.push(FromEngine::answer(seq, event));
    }

    pub(crate) fn told(&mut self, event: Event) {
        self.out.push(FromEngine::told(event));
    }

    pub(crate) fn error(&mut self, seq: u64, code: ErrorCode, message: &str) {
        self.answer(seq, Event::Error { code, message: message.to_owned() });
    }

    /// The account's state changed without the app asking: its next `targets` event goes out when the call ends.
    pub(crate) fn targets_changed(&mut self) {
        self.targets_changed = true;
    }

    /// A line for the engine's log. Fixed words and numbers only: never a token, an address, a login name or a persona.
    pub(crate) fn note(&mut self, line: &str) {
        (self.log)(line);
    }
}

pub fn descriptor() -> Descriptor {
    Descriptor {
        id: EngineId::new("steam").expect("steam is a valid engine id"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        capabilities: Capabilities {
            kind: EngineKind::Account,
            targets: TargetKind::Account,
            auth: vec![AuthMethod::Qr],
            presentation: Presentation::Nothing,
            features: BTreeSet::new(),
            video: None,
        },
    }
}

/// The settings page: no settings, one note on what Steam gives and what it keeps to itself.
pub fn schema() -> SettingsSchema {
    SettingsSchema {
        sections: vec![Section {
            title: "Library".to_owned(),
            fields: vec![Field {
                key: "limits".to_owned(),
                label: "What comes from Steam".to_owned(),
                help: Some(
                    "Last played, play time, genres and tags come from the Steam account. Steam's own Favorites and collections are kept by the Steam \
                     client and cannot be read from here."
                        .to_owned(),
                ),
                kind: FieldKind::Note,
            }],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, Rig, STEAMID, TempDir};
    use lumagame_core::engine::Manifest;
    use lumagame_core::ids::SessionId;
    use lumagame_core::ipc::{self, AuthState, ToEngine};
    use std::fs;

    fn target_id() -> TargetId {
        TargetId::new(STEAMID).unwrap()
    }

    fn error_of(events: &[FromEngine]) -> (Option<u64>, ErrorCode, &str) {
        match events {
            [FromEngine { re, event: Event::Error { code, message } }] => (*re, *code, message),
            other => panic!("expected one error, got {other:?}"),
        }
    }

    #[test]
    fn hello_answers_with_what_the_engine_is_and_a_page_the_app_accepts() {
        let mut rig = Rig::new();
        let events = rig.hello();
        assert_eq!(events, [FromEngine::answer(1, Event::Ready { descriptor: descriptor(), schema: schema() })]);
        let capabilities = descriptor().capabilities;
        assert_eq!((capabilities.kind, capabilities.targets, capabilities.presentation), (EngineKind::Account, TargetKind::Account, Presentation::Nothing));
        assert_eq!(capabilities.auth, [AuthMethod::Qr]);
        assert!(capabilities.features.is_empty() && capabilities.video.is_none());
        assert_eq!(schema().validate(), Ok(()));
        assert_eq!(rig.log.lines(), ["ready, signed out"]);
    }

    #[test]
    fn the_manifest_describes_the_same_engine_the_descriptor_does() {
        let manifest = Manifest::parse("steam", include_str!("../engine.json")).unwrap();
        assert_eq!((manifest.id, manifest.kind), (descriptor().id, EngineKind::Account));
        assert_eq!(manifest.exec.as_deref(), Some("bin/lumagame-engine-steam"));
        assert_eq!(manifest.license, "MIT");
    }

    #[test]
    fn anything_but_a_compatible_hello_first_ends_the_conversation() {
        let mut rig = Rig::new();
        let events = rig.send(Request::Targets);
        assert_eq!(error_of(&events), (Some(1), ErrorCode::Protocol, "The first request must be a compatible hello"));
        assert!(rig.adapter.finished());
        assert!(rig.send(Request::Targets).is_empty());

        let mut rig = Rig::new();
        let mut hello = rig.hello_request();
        hello.protocol = PROTOCOL + 1;
        let events = rig.send(Request::Hello(hello));
        assert_eq!(error_of(&events), (Some(1), ErrorCode::Protocol, "The first request must be a compatible hello"));
        assert!(rig.adapter.finished());
    }

    #[test]
    fn relative_directories_are_refused_and_a_second_hello_is_not_obeyed() {
        let mut rig = Rig::new();
        let mut hello = rig.hello_request();
        hello.data_dir = "data".into();
        let events = rig.send(Request::Hello(hello));
        assert_eq!(error_of(&events).1, ErrorCode::BadRequest);
        assert!(rig.adapter.finished());

        let mut rig = Rig::ready();
        let events = rig.send(Request::Hello(rig.hello_request()));
        assert_eq!(error_of(&events), (Some(2), ErrorCode::BadRequest, "Hello was already received"));
        assert!(!rig.adapter.finished());
        assert_eq!(rig.send(Request::Targets), [FromEngine::answer(3, Event::Targets { targets: Vec::new() })]);
    }

    #[test]
    fn requests_the_engine_has_no_use_for_are_refused_by_name_and_settings_are_ignored() {
        let mut rig = Rig::ready();
        let session = SessionId(1);
        let game = lumagame_core::ids::ProviderGameId::new("1").unwrap();
        let refused = [
            Request::Check { target: target_id(), game: game.clone() },
            Request::Launch { session, target: target_id(), game },
            Request::Stop { session, end_game: false },
            Request::Stats { session, on: true, pad_timing: false },
            Request::RoundTrip { session },
            Request::SteamOverlay { session },
            Request::Activity { session },
            Request::Wake { target: target_id() },
            Request::AddTarget { address: "x".into() },
            Request::AuthPin { pin: "1234".into() },
            Request::Auth { target: None, method: AuthMethod::DeviceCode },
        ];
        for request in refused {
            let events = rig.send(request.clone());
            assert_eq!(error_of(&events), (Some(rig.seq), ErrorCode::Unsupported, "This operation is not supported by Steam"), "{request:?}");
        }
        assert!(rig.send(Request::Settings { values: Default::default() }).is_empty());
        assert!(rig.send(Request::Action { key: "limits".into() }).is_empty());
        assert!(rig.http.sent().is_empty());
    }

    #[test]
    fn targets_list_the_account_with_its_state() {
        let mut rig = Rig::signed_in();
        let expected = Target { id: target_id(), name: "Rig".into(), detail: None, link: Link::Linked, reach: Reach::Online };
        assert_eq!(rig.send(Request::Targets), [FromEngine::answer(2, Event::Targets { targets: vec![expected] })]);

        let mut rig = Rig::new();
        rig.save_token(&fake::jwt(STEAMID, Some(fake::UNIX0 - 1)), STEAMID, None);
        rig.hello();
        let events = rig.send(Request::Targets);
        let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((targets[0].link, targets[0].name.as_str()), (Link::Expired, "Steam account"));
    }

    #[test]
    fn forget_deletes_everything_kept_and_is_safe_to_repeat() {
        let mut rig = Rig::signed_in();
        for name in ["library.json", "store.json"] {
            files::write(&rig.cache(), name, b"{}").unwrap();
        }
        let wrong = TargetId::new("76561198000000002").unwrap();
        assert_eq!(error_of(&rig.send(Request::Forget { target: wrong })).1, ErrorCode::BadRequest);
        assert!(rig.data().join("token.json").exists());

        let events = rig.send(Request::Forget { target: target_id() });
        assert_eq!(events, [FromEngine::answer(rig.seq, Event::Targets { targets: Vec::new() })]);
        assert!(!rig.data().join("token.json").exists());
        assert!(!rig.cache().join("library.json").exists() && !rig.cache().join("store.json").exists());
        assert_eq!(rig.send(Request::Targets), [FromEngine::answer(rig.seq, Event::Targets { targets: Vec::new() })]);
        assert_eq!(error_of(&rig.send(Request::Library { target: target_id(), cursor: None })).1, ErrorCode::BadRequest);
        // Again, signed out: still clean, still answered.
        let events = rig.send(Request::Forget { target: target_id() });
        assert_eq!(events, [FromEngine::answer(rig.seq, Event::Targets { targets: Vec::new() })]);
        // And it stays gone across a restart.
        rig.restart();
        rig.hello();
        assert_eq!(rig.send(Request::Targets), [FromEngine::answer(2, Event::Targets { targets: Vec::new() })]);
    }

    #[test]
    fn a_sign_in_that_cannot_be_deleted_stays_signed_in() {
        let mut rig = Rig::signed_in();
        // A directory where the file was: removing it fails with something other than "not found".
        fs::remove_file(rig.data().join("token.json")).unwrap();
        fs::create_dir(rig.data().join("token.json")).unwrap();
        let events = rig.send(Request::Forget { target: target_id() });
        assert_eq!(error_of(&events), (Some(2), ErrorCode::Internal, "The saved Steam sign-in could not be removed"));
        let events = rig.send(Request::Targets);
        let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(targets.len(), 1);
    }

    #[test]
    fn an_undecodable_sign_in_file_is_a_signed_out_engine_and_is_left_alone() {
        for (token, steamid) in [("not a token", STEAMID), ("a.b.c", STEAMID), (fake::jwt("76561198000000002", None).as_str(), STEAMID)] {
            let mut rig = Rig::new();
            rig.save_token(token, steamid, None);
            rig.hello();
            assert_eq!(rig.send(Request::Targets), [FromEngine::answer(2, Event::Targets { targets: Vec::new() })], "{token}");
            assert!(rig.data().join("token.json").exists());
        }
        let temp = TempDir::new();
        assert!(Account::load(temp.path(), 0).is_none());
    }

    /// Everything the engine sent or logged over a whole sign-in, a library refresh and a refresh that fails.
    #[test]
    fn no_token_login_name_or_address_with_one_leaves_the_engine() {
        let refresh = fake::jwt(STEAMID, Some(fake::UNIX0 + 86_400));
        let access = fake::jwt(STEAMID, Some(fake::UNIX0 + 3_600));
        let approved = format!(
            r#"{{"response":{{"refresh_token":"{refresh}","access_token":"POLLACCESSMARKER","account_name":"loginnamemarker","had_remote_interaction":true}}}}"#
        );
        let mut rig = Rig::ready();
        rig.http.on(
            "BeginAuthSessionViaQR",
            [fake::ok(r#"{"response":{"client_id":"42","challenge_url":"https://s.team/q/1/42","request_id":"cmVx+/==","interval":5.0}}"#)],
        );
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved)]);
        rig.http.on("steamcommunity.com", [fake::ok("<profile><steamID64>1</steamID64><steamID><![CDATA[Persona Name]]></steamID></profile>")]);
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
        rig.http.on("GetOwnedGames", [fake::ok(&fake::owned(&[(10, "Counter-Strike", 60, 1_700_000_000)]))]);
        rig.http.on("IStoreService", [fake::ok(r#"{"response":{"version_hash":"1","tags":[{"tagid":19,"name":"Action"}]}}"#)]);
        rig.http.on("IStoreBrowseService", [fake::ok(r#"{"response":{"store_items":[{"id":10,"success":1,"tagids":[19]}]}}"#)]);

        let mut events = rig.send(Request::Auth { target: None, method: AuthMethod::Qr });
        events.extend(rig.wake());
        events.extend(rig.send(Request::Library { target: target_id(), cursor: None }));
        rig.http.on("GetOwnedGames", [fake::down()]);
        rig.clock.advance_s(400);
        events.extend(rig.send(Request::Library { target: target_id(), cursor: None }));
        rig.http.on("GenerateAccessTokenForApp", [fake::eresult(15, r#"{"response":{}}"#)]);
        rig.clock.advance_s(4_000);
        events.extend(rig.send(Request::Library { target: target_id(), cursor: None }));

        assert!(events.iter().any(|e| matches!(e.event, Event::Auth { state: AuthState::Done { .. } })), "{events:?}");
        let sent = events.iter().map(ipc::encode).collect::<Vec<_>>().join("\n");
        let logged = rig.log.lines().join("\n");
        for secret in [refresh.as_str(), access.as_str(), "POLLACCESSMARKER", "loginnamemarker", "access_token", "Authorization", "refresh_token", "cmVx"] {
            assert!(!sent.contains(secret) && !logged.contains(secret), "{secret} leaked");
        }
        // The profile name and the id go to the app, which shows them, but not into a log.
        for private in ["Persona Name", STEAMID] {
            assert!(!logged.contains(private), "{private} was logged: {logged}");
        }
        assert!(sent.contains("https://s.team/q/1/42"), "the code's address is the one thing meant to be sent");
        assert!(!events.iter().any(|e| matches!(&e.event, Event::Auth { state: AuthState::Failed { .. } })));
    }

    #[test]
    fn a_line_from_the_app_decodes_into_what_handle_takes() {
        let line = ipc::encode(&ToEngine { seq: 7, request: Request::Targets });
        let message: ToEngine = ipc::decode(&line).unwrap();
        let mut rig = Rig::ready();
        assert_eq!(rig.adapter.handle(message.seq, message.request), [FromEngine::answer(7, Event::Targets { targets: Vec::new() })]);
    }
}
