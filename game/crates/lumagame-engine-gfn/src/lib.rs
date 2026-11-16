//! LumaGame's protocol-1 adapter for the unchanged OpenNOW core (protocol 5) and streamer (protocol 7).
//! Account files stay beneath Hello.data_dir; migrated prefs.json supplies settings defaults, never credentials to the shell.
//! The deterministic adapter accepts JSON lines and an injected monotonic clock so a fake core exercises the real protocol path.
mod catalog;
mod settings;
pub mod runtime;

use catalog::Selection;
use lumagame_core::engine::{AuthMethod, Capabilities, Codec, Descriptor, EngineKind, Feature, Presentation, TargetKind, VideoCaps};
use lumagame_core::ids::{EngineId, ProviderGameId, SessionId, TargetId};
use lumagame_core::ipc::*;
use lumagame_core::settings::{SettingsSchema, Values};
use serde_json::{Value, json};
use lumatv_gfn_pad::{PadState, menu::{MenuKey, MenuKeys}};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone)]
enum Purpose {
    Hello,
    Setting,
    Detect,
    Restore,
    Subscription,
    Regions,
    AuthStart,
    AuthPoll,
    AuthComplete,
    AuthCancel,
    Logout,
    Library { target: TargetId, cursor: Option<String> },
    Inspect { target: TargetId, game: ProviderGameId, launch: bool },
    RefreshLaunch,
    Create,
    Remote,
    Poll,
    Start,
    RetryStop,
    StopStream,
    StopSeat,
    Pause,
}

#[derive(Debug, Clone)]
struct Job {
    method: &'static str,
    params: Value,
    purpose: Purpose,
    seq: Option<u64>,
    epoch: u64,
}

struct Pending {
    id: String,
    job: Job,
    since: u64,
}

struct Auth {
    seq: u64,
    attempt: String,
    due: u64,
    interval: u64,
    deadline: u64,
}

struct Launch {
    id: SessionId,
    seq: u64,
    selection: Selection,
    cloud: Value,
    started: bool,
    running: bool,
    stale_retried: bool,
    busy_retried: bool,
    claimed: bool,
    retry_stopping: bool,
    stopping: bool,
    end_game: bool,
    stats: bool,
    pad_timing: bool,
    poll_due: u64,
    values: Values,
    /// The app's overlay has the pads: the streamer pauses their input and hands the reports to [`Adapter::pad_line`].
    overlay: bool,
    menus: BTreeMap<u64, MenuKeys>,
    /// The latest legs the streamer and the pads reported, for `round-trip`.
    trip: RoundTrip,
    pad: Option<(PadStats, u64)>,
}

/// Output batches preserve ordering: session.create is acknowledged before its stream is started.
#[derive(Default)]
pub struct Output {
    pub core: Vec<String>,
    pub app: Vec<FromEngine>,
}

pub struct Adapter {
    hello: Hello,
    schema: SettingsSchema,
    values: Values,
    streamer: String,
    hello_seq: u64,
    ready: bool,
    failed: bool,
    now: u64,
    next_id: u64,
    epoch: u64,
    queue: VecDeque<Job>,
    pending: Option<Pending>,
    output: Output,
    scope: Value,
    target: Option<Target>,
    runtime: Value,
    region: Option<(String, String)>,
    catalog_revision: Value,
    catalog_context: Value,
    cursors: BTreeSet<String>,
    selections: BTreeMap<ProviderGameId, Selection>,
    auth: Option<Auth>,
    launch: Option<Launch>,
}

impl Adapter {
    pub fn new(hello: Hello, seq: u64, prefs: &Value, streamer: String) -> Self {
        let schema = settings::schema(prefs);
        let values = schema.resolve(&hello.settings);
        let mut this = Self {
            hello, schema, values, streamer, hello_seq:seq, ready:false,failed:false,now:0,next_id:1,epoch:0,
            queue:VecDeque::new(),pending:None,output:Output::default(),scope:Value::Null,target:None,
            runtime:Value::Null,region:None,catalog_revision:Value::Null,catalog_context:Value::Null,
            cursors:BTreeSet::new(),selections:BTreeMap::new(),auth:None,launch:None,
        };
        this.enqueue("core.hello", json!({"protocolVersion":5,"shell":"lumagame","shellVersion":env!("CARGO_PKG_VERSION")}), Purpose::Hello, Some(seq));
        this
    }

    pub fn failed(&self) -> bool { self.failed }
    pub fn pad_stats_on(&self) -> bool { self.launch.as_ref().is_some_and(|l| l.running && l.pad_timing && !l.stopping) }
    pub fn data_dir(&self) -> &str { &self.hello.data_dir }
    pub fn pad_prefs(&self) -> Value {
        settings::pad_prefs(self.launch.as_ref().map_or(&self.values, |l| &l.values))
    }

    pub fn drain(&mut self) -> Output { std::mem::take(&mut self.output) }

    fn emit(&mut self, seq: Option<u64>, event: Event) { self.output.app.push(FromEngine { re:seq,event }); }
    fn error(&mut self, seq: Option<u64>, code: ErrorCode, message: impl Into<String>) {
        self.emit(seq, Event::Error { code,message:message.into() });
    }
    fn enqueue(&mut self, method: &'static str, params: Value, purpose: Purpose, seq: Option<u64>) {
        self.queue.push_back(Job { method, params, purpose, seq, epoch:self.epoch });
        self.pump();
    }
    fn pump(&mut self) {
        if self.pending.is_some() || self.failed { return; }
        let Some(job) = self.queue.pop_front() else { return; };
        let id = self.next_id.to_string();
        self.next_id += 1;
        self.output.core.push(encode(&json!({"type":"request","id":id,"method":job.method,"params":job.params})));
        self.pending = Some(Pending { id, job, since:self.now });
    }
    fn queued(&self, method: &str) -> bool {
        self.pending.as_ref().is_some_and(|p| p.job.method == method) || self.queue.iter().any(|j| j.method == method)
    }

    pub fn request(&mut self, message: ToEngine) {
        let seq = Some(message.seq);
        if !self.ready {
            self.error(seq, ErrorCode::BadRequest, "The GeForce NOW core is not ready");
            return;
        }
        match message.request {
            Request::Hello(_) => self.error(seq, ErrorCode::BadRequest, "Hello was already received"),
            Request::Settings { values } => {
                if values.iter().any(|(key,value)| self.schema.check(key,value).is_err()) {
                    self.error(seq, ErrorCode::BadRequest, "A GeForce NOW setting is invalid");
                } else { self.values = self.schema.resolve(&values); }
            }
            Request::Targets => {
                self.enqueue("auth.session.get", json!({}), Purpose::Restore, seq);
            }
            Request::Auth { target, method:AuthMethod::DeviceCode } if target.as_ref().is_none_or(|t| self.matches_target(t)) => {
                if self.auth.is_some() {
                    self.error(seq, ErrorCode::BadRequest, "A sign-in is already in progress");
                } else { self.begin_auth(message.seq); }
            }
            Request::AuthCancel => self.cancel_auth(message.seq),
            Request::Forget { target } if self.matches_target(&target) => self.logout(seq),
            Request::Action { key } if key == "sign-out" => self.logout(seq),
            Request::Library { target, cursor } if self.matches_target(&target) => {
                let mut params = json!({"limit":60,"cursor":cursor.as_deref().unwrap_or("")});
                if let Some(cursor) = &cursor {
                    if !self.cursors.contains(cursor) {
                        self.error(seq, ErrorCode::BadRequest, "This library cursor is no longer current");
                        return;
                    }
                    params["catalogRevision"] = self.catalog_revision.clone();
                    params["catalogContext"] = self.catalog_context.clone();
                } else {
                    self.cursors.clear();
                }
                self.enqueue("catalog.library.list", params, Purpose::Library { target,cursor }, seq);
            }
            Request::Check { target,game } if self.matches_target(&target) => self.inspect(target, game, false, seq),
            Request::Launch { session,target,game } => {
                if self.launch.is_some() {
                    self.emit(seq, Event::Session { session,state:SessionState::Failed { reason:FailReason::Busy,message:"A game is already active".into() } });
                    return;
                }
                let selection = match self.selections.get(&game).cloned().map(Ok).unwrap_or_else(|| catalog::parse_provider(&game)) {
                    Ok(s) => s,
                    Err(e) => { self.emit(seq, Event::Session { session,state:SessionState::Failed { reason:FailReason::NotInstalled,message:e } }); return; }
                };
                self.launch = Some(Launch {
                    id:session,seq:message.seq,selection,cloud:Value::Null,started:false,running:false,stale_retried:false,
                    busy_retried:false,claimed:false,retry_stopping:false,stopping:false,end_game:false,stats:false,pad_timing:false,poll_due:0,values:self.values.clone(),
                    overlay:false,menus:BTreeMap::new(),trip:RoundTrip::default(),pad:None,
                });
                self.session(SessionState::Checking);
                if self.target.is_none() {
                    self.session(SessionState::SignIn);
                    if self.auth.is_none() { self.begin_auth(message.seq); }
                } else if !self.matches_target(&target) {
                    self.finish(SessionState::Failed { reason:FailReason::Auth,message:"The account is no longer linked".into() });
                } else { self.inspect(target,game,true,seq); }
            }
            Request::Stop { session,end_game } if self.launch.as_ref().is_some_and(|l| l.id == session) => {
                self.stop(message.seq, end_game);
            }
            Request::Route { session,input } if self.launch.as_ref().is_some_and(|l| l.id == session && l.running) => {
                if let Some(l) = &mut self.launch { l.overlay = input == InputRoute::App; l.menus.clear(); }
                self.enqueue("streamer.input.pause",json!({"paused":input == InputRoute::App}),Purpose::Pause,seq);
            }
            Request::Stats { session,on,pad_timing } if self.launch.as_ref().is_some_and(|l| l.id == session) => {
                if let Some(l) = &mut self.launch { l.stats = on; l.pad_timing = on && pad_timing; if !l.pad_timing { l.pad = None; } }
            }
            Request::RoundTrip { session } if self.launch.as_ref().is_some_and(|l| l.id == session && l.running) => {
                let trip = self.launch.as_ref().map(|l| l.trip.clone()).unwrap_or_default();
                self.emit(seq,Event::RoundTrip { session,trip });
            }
            Request::Library { .. } | Request::Check { .. } | Request::Forget { .. } | Request::Stop { .. } | Request::Route { .. } | Request::Stats { .. }
            | Request::RoundTrip { .. } => {
                self.error(seq,ErrorCode::BadRequest,"The target or session is not current");
            }
            _ => self.error(seq,ErrorCode::Unsupported,"This operation is not supported by GeForce NOW"),
        }
    }

    fn matches_target(&self, target: &TargetId) -> bool { self.target.as_ref().is_some_and(|t| &t.id == target) }
    fn targets(&mut self, seq: Option<u64>) { self.emit(seq,Event::Targets { targets:self.target.iter().cloned().collect() }); }
    /// One datagram from the streamer's pad bridge (`LUMATV_GFN_PAD_SIDE`). The PS button asks for the overlay, as it does on every
    /// engine; while the overlay has the pads their reports become its menu keys.
    pub fn pad_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        let Some(l) = self.launch.as_mut().filter(|l| l.running && !l.stopping) else { return };
        let session = l.id;
        match v["pad"].as_str() {
            Some("guide") if v["pressed"] == true => self.emit(None,Event::Overlay { session }),
            Some("report") if l.overlay => {
                let axis = |k:&str| v[k].as_i64().and_then(|n| i16::try_from(n).ok()).unwrap_or(0);
                let state = PadState {
                    buttons:v["buttons"].as_u64().and_then(|n| u16::try_from(n).ok()).unwrap_or(0),
                    left_x:axis("lx"),left_y:axis("ly"),right_x:axis("rx"),right_y:axis("ry"),..PadState::default()
                };
                let mut keys = Vec::new();
                l.menus.entry(v["slot"].as_u64().unwrap_or(0)).or_default().feed(&state,|k| keys.push(k));
                for key in keys {
                    let key = match key {MenuKey::Up=>PadKey::Up,MenuKey::Down=>PadKey::Down,MenuKey::Left=>PadKey::Left,MenuKey::Right=>PadKey::Right,MenuKey::Ok=>PadKey::Ok,MenuKey::Back=>PadKey::Back,MenuKey::Menu=>PadKey::Menu};
                    for down in [true,false] { self.emit(None,Event::Key { key,down }); }
                }
            }
            Some("latency") => {
                let ms = |k:&str| v[k].as_f64().map(|us| (us/1000.0) as f32);
                (l.trip.presses,l.trip.press_p50_ms,l.trip.press_p99_ms) = (v["n"].as_u64().and_then(|n| u32::try_from(n).ok()),ms("p50"),ms("p99"));
            }
            Some("timing") if l.pad_timing => {
                if let Ok(stats) = serde_json::from_value::<PadStats>(v) { l.pad = Some((stats, self.now)); }
            }
            _ => {}
        }
    }
    fn session(&mut self, state: SessionState) {
        if let Some(l) = &self.launch { self.emit(Some(l.seq),Event::Session { session:l.id,state }); }
    }
    fn finish(&mut self, state: SessionState) {
        self.session(state);
        self.launch = None;
        self.queue.retain(|j| !matches!(j.purpose,Purpose::Create|Purpose::Poll|Purpose::Start|Purpose::RetryStop|Purpose::StopStream|Purpose::StopSeat|Purpose::RefreshLaunch|Purpose::Inspect{launch:true,..}));
    }
    fn inspect(&mut self, target: TargetId, game: ProviderGameId, launch: bool, seq: Option<u64>) {
        let selection = self.selections.get(&game).cloned().map(Ok).unwrap_or_else(|| catalog::parse_provider(&game));
        match selection {
            Ok(s) => self.enqueue("catalog.launch.inspect",json!({"appId":s.parent,"variantId":s.variant}),Purpose::Inspect { target,game,launch },seq),
            Err(e) => self.error(seq,ErrorCode::BadRequest,e),
        }
    }
    fn begin_auth(&mut self, seq: u64) {
        self.auth = Some(Auth { seq,attempt:String::new(),due:0,interval:5000,deadline:0 });
        self.enqueue("auth.device.start",json!({}),Purpose::AuthStart,Some(seq));
    }
    fn cancel_auth(&mut self, seq: u64) {
        if let Some(a) = self.auth.take() {
            self.queue.retain(|j| !matches!(j.purpose,Purpose::AuthStart|Purpose::AuthPoll|Purpose::AuthComplete));
            if !a.attempt.is_empty() { self.enqueue("auth.device.cancel",json!({"attemptId":a.attempt}),Purpose::AuthCancel,Some(seq)); }
            self.emit(Some(seq),Event::Auth { state:AuthState::Failed { reason:AuthFail::Cancelled,message:"Sign-in cancelled".into() } });
            if self.launch.is_some() { self.finish(SessionState::Failed { reason:FailReason::Auth,message:"Sign-in cancelled".into() }); }
        }
    }
    fn logout(&mut self, seq: Option<u64>) {
        if self.launch.is_some() {
            self.error(seq,ErrorCode::BadRequest,"Stop the game before signing out");
            return;
        }
        self.enqueue("auth.accounts.logoutAll",json!({}),Purpose::Logout,seq);
    }
    fn settings(&mut self) {
        let values = self.launch.as_ref().map_or(&self.values, |l| &l.values);
        for (key,value) in settings::core_settings(values,&self.hello.display) {
            self.enqueue("settings.set",json!({"key":key,"value":value}),Purpose::Setting,None);
        }
    }
    fn create(&mut self) {
        let Some(l) = &self.launch else { return; };
        if l.stopping { return; }
        let s = &l.selection;
        let mut params = json!({"appId":s.variant,"catalogAppId":s.parent,"variantId":s.variant,"scope":self.scope,
            "selectionIdentity":format!("{}:{}",s.parent,s.variant),"title":s.title,"store":s.store,"accountLinked":s.owned,
            "appLaunchMode":"gamepadFriendly","runtimeCapabilities":self.runtime,"maxEntitledFps":self.hello.display.refresh_hz.min(120)});
        if let Some((zone,url)) = &self.region { params["zone"] = json!(zone); params["streamingBaseUrl"] = json!(url); }
        self.enqueue("session.create",params,Purpose::Create,None);
    }
    fn start(&mut self) {
        let Some(l) = &self.launch else { return; };
        if l.stopping { return; }
        self.enqueue("streamer.start",json!({"session":l.cloud,"surface":{"visible":true,"rect":{"x":0,"y":0,"width":self.hello.display.width,"height":self.hello.display.height}}}),Purpose::Start,None);
    }
    fn stop(&mut self, seq: u64, end_game: bool) {
        let Some(l) = &mut self.launch else { return; };
        l.stopping = true; l.seq = seq; l.end_game = end_game;
        self.queue.retain(|j| !matches!(j.purpose,Purpose::Create|Purpose::Remote|Purpose::Poll|Purpose::Start|Purpose::RetryStop|Purpose::RefreshLaunch|Purpose::Inspect{launch:true,..}));
        if self.auth.is_some() { self.cancel_auth(seq); return; }
        self.enqueue("streamer.stop",json!({"reason":"the viewer left the stream"}),Purpose::StopStream,Some(seq));
    }

    /// Clocks are monotonic milliseconds; no test sleeps and no wall-clock change affects a deadline.
    pub fn tick(&mut self, now: u64) {
        self.now = now;
        if let Some(p) = &self.pending {
            let timeout = if matches!(p.job.purpose,Purpose::Hello) {5000} else {120_000};
            if now.saturating_sub(p.since) >= timeout {
                self.core_gone("The GeForce NOW core did not answer in time");
                return;
            }
        }
        if let Some(a) = &self.auth {
            if a.deadline > 0 && now >= a.deadline {
                let seq = a.seq;
                let attempt = a.attempt.clone();
                self.auth_failure(AuthFail::Expired,"The sign-in code expired");
                self.enqueue("auth.device.cancel",json!({"attemptId":attempt}),Purpose::AuthCancel,Some(seq));
            } else if a.due > 0 && now >= a.due && !self.queued("auth.device.poll") && !self.queued("auth.device.complete") {
                self.enqueue("auth.device.poll",json!({"attemptId":a.attempt}),Purpose::AuthPoll,Some(a.seq));
            }
        }
        if let Some(l) = &self.launch {
            if !l.stopping && !l.started && l.poll_due > 0 && now >= l.poll_due && !self.queued("session.poll") {
                self.enqueue("session.poll",json!({"sessionId":l.cloud["sessionId"]}),Purpose::Poll,None);
            }
        }
    }

    pub fn core_gone(&mut self, message: &str) {
        self.failed = true;
        self.queue.clear(); self.pending = None;
        if self.auth.take().is_some() {
            self.emit(None,Event::Auth { state:AuthState::Failed { reason:AuthFail::Network,message:message.into() } });
        }
        if self.launch.is_some() { self.finish(SessionState::Failed { reason:FailReason::Engine,message:message.into() }); }
        else { self.error(None,ErrorCode::Internal,message); }
    }

    pub fn core_line(&mut self, line: &str) {
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => { self.core_gone("The GeForce NOW core sent invalid JSON"); return; }
        };
        match value["type"].as_str() {
            Some("event") => self.event(value["name"].as_str().unwrap_or(""),&value["payload"]),
            Some("response") => {
                let Some(pending) = self.pending.take() else { return; };
                if value["id"].as_str() != Some(&pending.id) {
                    self.pending = Some(pending);
                    return;
                }
                if pending.job.epoch != self.epoch { self.pump(); return; }
                if value["ok"] == true {
                    let result = &value["result"];
                    if self.foreign(result) {
                        self.error(pending.job.seq,ErrorCode::BadRequest,"The account changed while the request was running");
                        if self.launch.is_some() && matches!(pending.job.purpose,Purpose::Create|Purpose::Poll|Purpose::Start|Purpose::Inspect { launch:true,.. }) {
                            self.finish(SessionState::Failed { reason:FailReason::Auth,message:"The account changed while the request was running".into() });
                        }
                    } else {
                        if matches!(pending.job.purpose,Purpose::Create) {
                            self.output.core.push(encode(&json!({"type":"ack","id":pending.id})));
                        }
                        self.adopt_scope(&result["scope"]);
                        self.response(pending.job,result);
                    }
                } else {
                    self.rejected(pending.job,value["error"]["code"].as_str().unwrap_or("unknown"));
                }
            }
            _ => self.core_gone("The GeForce NOW core sent an invalid message"),
        }
        self.pump();
    }

    fn foreign(&self, value: &Value) -> bool {
        let scope = &value["scope"];
        scope.is_object() && self.scope.is_object() && (scope["userId"] != self.scope["userId"] || scope["providerIdpId"] != self.scope["providerIdpId"])
    }
    fn adopt_scope(&mut self, scope: &Value) {
        if scope.is_object() && (self.scope.is_null() || (scope["userId"] == self.scope["userId"] && scope["providerIdpId"] == self.scope["providerIdpId"] && scope["generation"].as_u64() >= self.scope["generation"].as_u64())) {
            self.scope = scope.clone();
        }
    }
    fn envelope_scope(r: &Value) -> Value {
        json!({"generation":r["generation"],"providerIdpId":r["session"]["provider"]["idpId"],"userId":r["session"]["user"]["userId"]})
    }
    fn account(&mut self, r: &Value, seq: Option<u64>, completed: bool) {
        if !r["session"].is_object() {
            self.target = None; self.scope = Value::Null;
            self.targets(seq);
            if completed { self.auth_failure(AuthFail::Network,"Sign-in returned no account"); }
            return;
        }
        let scope = Self::envelope_scope(r);
        if self.scope.is_object() && scope["generation"].as_u64() < self.scope["generation"].as_u64() { return; }
        if self.scope.is_object() && (scope["userId"] != self.scope["userId"] || scope["providerIdpId"] != self.scope["providerIdpId"]) {
            self.epoch += 1;
            self.queue.clear(); self.selections.clear(); self.cursors.clear();
            if self.launch.is_some() {
                self.finish(SessionState::Failed { reason:FailReason::Auth,message:"The signed-in account changed".into() });
                self.enqueue("streamer.stop",json!({"reason":"the account changed"}),Purpose::Pause,None);
            }
        }
        let user = r["session"]["user"]["userId"].as_str().unwrap_or("");
        let provider = r["session"]["provider"]["idpId"].as_str().unwrap_or("");
        if user.is_empty() || provider.is_empty() { self.core_gone("The GeForce NOW account identity is missing"); return; }
        let natural = format!("account:{}:{provider}{user}", provider.len());
        let id = TargetId::new(natural).or_else(|_| {
            let bytes = serde_json::to_vec(&[provider,user]).unwrap();
            TargetId::new(format!("account-{}",bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
        });
        let Ok(id) = id else { self.core_gone("The GeForce NOW account identity is invalid"); return; };
        self.scope = scope;
        self.target = Some(Target { id:id.clone(),name:r["session"]["user"]["displayName"].as_str().unwrap_or("GeForce NOW").into(),
            detail:r["session"]["user"]["membershipTier"].as_str().map(str::to_owned),link:Link::Linked,reach:Reach::Online });
        self.targets(seq);
        if completed {
            if let Some(a) = self.auth.take() { self.emit(Some(a.seq),Event::Auth { state:AuthState::Done { target:id } }); }
        }
    }

    fn response(&mut self, job: Job, r: &Value) {
        match job.purpose {
            Purpose::Hello => {
                if r["protocolVersion"].as_u64() != Some(5) { self.core_gone("The GeForce NOW core protocol is incompatible"); return; }
                self.enqueue("settings.set",json!({"key":"nativeStreamerExecutablePath","value":self.streamer}),Purpose::Setting,None);
                self.settings();
                self.enqueue("streamer.detect",json!({}),Purpose::Detect,None);
                self.enqueue("auth.session.get",json!({}),Purpose::Restore,None);
            }
            Purpose::Setting => {}
            Purpose::Detect => {
                if r["available"] != true || r["capabilities"]["protocolVersion"].as_u64() != Some(7) {
                    self.core_gone("The GeForce NOW streamer is unavailable or incompatible"); return;
                }
                self.runtime = r["capabilities"].clone();
                self.runtime["nativeHdrSupported"] = json!(self.hello.display.hdr10);
                if self.hello.display.hdr10 {
                    self.runtime["nativeHdrDisplay"] = json!({"minimumNits":0,"maximumNits":1000,"maximumFullFrameNits":400});
                }
                self.ready = true;
                self.emit(Some(self.hello_seq),Event::Ready { descriptor:descriptor(),schema:self.schema.clone() });
            }
            Purpose::Restore => {
                self.account(r,job.seq,false);
                if self.target.is_some() {
                    self.enqueue("account.subscription.get",json!({}),Purpose::Subscription,None);
                    self.enqueue("network.regions.list",json!({}),Purpose::Regions,None);
                }
            }
            Purpose::Subscription => {
                if let Some(tier) = r["subscription"]["membershipTier"].as_str() {
                    if let Some(t) = &mut self.target { t.detail = Some(tier.into()); }
                    self.targets(None);
                }
            }
            Purpose::Regions => {
                self.region = r["regions"].as_array().and_then(|a| a.first()).and_then(|v| Some((v["name"].as_str()?.into(),v["url"].as_str()?.into())));
            }
            Purpose::AuthStart => {
                let Some(a) = &mut self.auth else { return; };
                let Some(attempt) = r["attemptId"].as_str().filter(|s| !s.is_empty()) else { self.auth_failure(AuthFail::Network,"Sign-in returned no attempt"); return; };
                let Some(url) = r["verificationUri"].as_str().filter(|s| s.starts_with("https://")) else { self.auth_failure(AuthFail::Network,"Sign-in returned no secure address"); return; };
                let Some(code) = r["userCode"].as_str().filter(|s| !s.is_empty()) else { self.auth_failure(AuthFail::Network,"Sign-in returned no code"); return; };
                a.attempt = attempt.into();
                a.interval = r["intervalSeconds"].as_u64().unwrap_or(5).clamp(1,3600)*1000;
                a.due = self.now+a.interval;
                let expires = r["expiresAt"].as_u64().map(|n| n.saturating_sub(runtime::unix_ms()).div_ceil(1000)).unwrap_or(600).clamp(1,3600) as u32;
                a.deadline = self.now+u64::from(expires)*1000;
                let seq = a.seq;
                self.emit(Some(seq),Event::Auth { state:AuthState::DeviceCode { url:url.into(),code:code.into(),
                    qr:r["verificationUriComplete"].as_str().filter(|s| s.starts_with("https://")).map(str::to_owned),expires_s:expires } });
            }
            Purpose::AuthPoll => {
                let Some(a) = &mut self.auth else { return; };
                match r["status"].as_str() {
                    Some("authorized") => {
                        let params = json!({"attemptId":a.attempt}); let seq = a.seq;
                        a.due = 0;
                        self.enqueue("auth.device.complete",params,Purpose::AuthComplete,Some(seq));
                    }
                    Some("expired") => self.auth_failure(AuthFail::Expired,"The sign-in code expired"),
                    Some("access_denied") => self.auth_failure(AuthFail::Denied,"Sign-in was declined"),
                    Some("pending"|"slow_down") => { a.due = self.now+r["retryAfterMs"].as_u64().unwrap_or(a.interval).max(1000); }
                    _ => self.auth_failure(AuthFail::Network,"Sign-in could not be completed"),
                }
            }
            Purpose::AuthComplete => {
                if self.auth.is_none() {
                    // Completion may already have crossed the core's commit fence when cancelled.
                    self.enqueue("auth.session.get",json!({}),Purpose::Restore,None);
                    return;
                }
                self.account(r,job.seq,true);
                if self.target.is_some() {
                    self.enqueue("account.subscription.get",json!({}),Purpose::Subscription,None);
                    self.enqueue("network.regions.list",json!({}),Purpose::Regions,None);
                    self.resume_launch();
                }
            }
            Purpose::AuthCancel => { self.enqueue("auth.session.get",json!({}),Purpose::Restore,None); }
            Purpose::Logout => {
                self.epoch += 1; self.queue.clear(); self.auth = None; self.target = None; self.scope = Value::Null;
                self.selections.clear(); self.cursors.clear(); self.region = None;
                self.targets(job.seq);
            }
            Purpose::Library { target,cursor } => self.library(r,target,cursor,job.seq),
            Purpose::Inspect { target,game,launch } => {
                let readiness = match r["decision"]["status"].as_str() {
                    Some("ready") => Readiness::Ready,
                    Some("link_required") => Readiness::SignIn,
                    Some("ownership_required"|"subscription_required"|"unavailable") => Readiness::Install,
                    _ => Readiness::Unknown,
                };
                let expected = catalog::parse_provider(&game).expect("an inspected id was already validated");
                if r["appId"].as_str() != Some(&expected.parent) || r["variantId"].as_str() != Some(&expected.variant) {
                    if launch { self.finish(SessionState::Failed { reason:FailReason::Engine,message:"The catalog returned a different game".into() }); }
                    else { self.error(job.seq,ErrorCode::Internal,"The catalog returned a different game"); }
                    return;
                }
                if launch {
                    if self.launch.as_ref().is_none_or(|l| l.stopping) { return; }
                    if readiness != Readiness::Ready {
                        self.finish(SessionState::Failed { reason:if readiness == Readiness::SignIn {FailReason::Auth} else {FailReason::NotEntitled},
                            message:r["decision"]["message"].as_str().unwrap_or("This game is not ready to launch").into() });
                    } else {
                        if let Ok(entries) = catalog::selections(&r["game"]) {
                            if let Some((_,selection)) = entries.into_iter().find(|(e,_)| e.id == game) {
                                if let Some(l) = &mut self.launch { l.selection = selection; }
                            }
                        }
                        self.settings(); self.create();
                    }
                } else { self.emit(job.seq,Event::Readiness { target,game,readiness }); }
            }
            Purpose::RefreshLaunch => {
                self.account(r,None,false);
                self.resume_launch();
            }
            Purpose::Create | Purpose::Poll => self.session_update(r),
            Purpose::Remote => self.claim(r),
            Purpose::Start => {
                if let Some(l) = &mut self.launch {
                    if !l.stopping { l.running = true; self.running(); }
                }
                self.stats(r);
            }
            Purpose::RetryStop => {
                if let Some(l) = &mut self.launch { l.retry_stopping = false; }
                self.start();
            }
            Purpose::StopStream => {
                let Some(l) = &self.launch else { return; };
                if l.end_game && l.cloud["sessionId"].is_string() {
                    self.enqueue("session.stop",json!({"sessionId":l.cloud["sessionId"],"streamingBaseUrl":l.cloud["streamingBaseUrl"],"serverIp":l.cloud["serverIp"]}),Purpose::StopSeat,job.seq);
                } else { self.finish(SessionState::Ended { reason:EndReason::Suspended }); }
            }
            Purpose::StopSeat => self.finish(SessionState::Ended { reason:EndReason::Stopped }),
            Purpose::Pause => {}
        }
    }

    fn auth_failure(&mut self, reason: AuthFail, message: &str) {
        if let Some(a) = self.auth.take() { self.emit(Some(a.seq),Event::Auth { state:AuthState::Failed { reason,message:message.into() } }); }
        if self.launch.is_some() { self.finish(SessionState::Failed { reason:FailReason::Auth,message:message.into() }); }
    }
    fn resume_launch(&mut self) {
        let (Some(l),Some(t)) = (&self.launch,&self.target) else { return; };
        let game = ProviderGameId::new(serde_json::to_string(&[&l.selection.parent,&l.selection.variant]).unwrap()).unwrap();
        self.inspect(t.id.clone(),game,true,Some(l.seq));
    }
    fn library(&mut self, r: &Value, target: TargetId, cursor: Option<String>, seq: Option<u64>) {
        let result = (|| {
            let games = r["games"].as_array().ok_or("The library response has no games")?;
            let mut entries = Vec::new();
            let mut selections = Vec::new();
            for game in games {
                for (entry, selection) in catalog::selections(game)? { entries.push(entry); selections.push(selection); }
            }
            let next = r["nextCursor"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
            if r["hasNextPage"] == true && next.is_none() { return Err("The library response has no continuation".into()); }
            if let Some(n) = &next {
                if n.len() > 4096 || cursor.as_ref() == Some(n) || self.cursors.contains(n) { return Err("The library cursor repeated".into()); }
                if r["catalogRevision"].is_null() || r["catalogContext"].is_null() { return Err("The library continuation has no context".into()); }
            }
            Ok::<_,String>((entries,selections,next))
        })();
        match result {
            Ok((entries,selections,next)) => {
                for (entry,selection) in entries.iter().zip(selections) { self.selections.insert(entry.id.clone(),selection); }
                self.catalog_revision = r["catalogRevision"].clone(); self.catalog_context = r["catalogContext"].clone();
                if let Some(n) = &next { self.cursors.insert(n.clone()); }
                self.emit(seq,Event::Library { target,entries,next });
            }
            Err(e) => self.error(seq,ErrorCode::Internal,e),
        }
    }
    fn rejected(&mut self, job: Job, code: &str) {
        match job.purpose {
            Purpose::Regions | Purpose::Subscription => {}
            Purpose::Restore => { self.targets(job.seq); }
            Purpose::AuthPoll => { if let Some(a) = &mut self.auth { a.due = self.now+a.interval; } }
            Purpose::AuthStart | Purpose::AuthComplete => self.auth_failure(AuthFail::Network,"GeForce NOW could not complete sign-in"),
            Purpose::Create if code == "stale_account" && self.launch.as_ref().is_some_and(|l| !l.stale_retried && !l.stopping) => {
                if let Some(l) = &mut self.launch { l.stale_retried = true; }
                self.enqueue("auth.session.get",json!({}),Purpose::RefreshLaunch,None);
            }
            // The same account still holds a seat, usually the game this TV left without ending it: take it back once.
            Purpose::Create if code == "session_conflict" && self.launch.as_ref().is_some_and(|l| !l.claimed && !l.stopping) => {
                if let Some(l) = &mut self.launch { l.claimed = true; }
                self.enqueue("session.remote.list",json!({}),Purpose::Remote,None);
            }
            Purpose::Start if code == "streamer_busy" && self.launch.as_ref().is_some_and(|l| !l.busy_retried && !l.stopping) => {
                if let Some(l) = &mut self.launch { l.busy_retried = true; l.retry_stopping = true; }
                self.enqueue("streamer.stop",json!({"reason":"a finished stream still held the engine"}),Purpose::RetryStop,None);
            }
            Purpose::Poll if code == "busy" || code == "session_update_busy" => {
                if let Some(l) = &mut self.launch { l.poll_due = self.now+1000; }
            }
            Purpose::RetryStop => {
                if let Some(l) = &mut self.launch { l.retry_stopping = false; }
                self.start();
            }
            Purpose::Create | Purpose::Remote | Purpose::Poll | Purpose::Start | Purpose::RefreshLaunch | Purpose::Inspect { launch:true,.. } | Purpose::StopStream | Purpose::StopSeat => {
                let reason = if code == "stale_account" {FailReason::Auth} else if matches!(code,"streamer_busy"|"session_conflict") {FailReason::Busy} else {FailReason::Engine};
                let message = if code == "stale_account" {"This game could not be started for the signed-in account. Sign out and sign in again."}
                    else if code == "streamer_busy" {"The last stream has not closed yet. Try playing again."}
                    else if code == "session_conflict" {"Another GeForce NOW game is still running on this account. End it, or wait a few minutes, then play again."}
                    else {"GeForce NOW could not complete the session request"};
                self.finish(SessionState::Failed { reason,message:message.into() });
                self.enqueue("streamer.stop",json!({"reason":"the session request failed"}),Purpose::Pause,None);
            }
            Purpose::Hello | Purpose::Detect | Purpose::Setting => self.core_gone("GeForce NOW could not initialize the core and streamer"),
            _ => self.error(job.seq,ErrorCode::Internal,format!("GeForce NOW refused {} ({code})",job.method)),
        }
    }
    fn claim(&mut self, r: &Value) {
        let Some(l) = &self.launch else { return; };
        if l.stopping { return; }
        let variant = l.selection.variant.clone();
        let same = |s: &&Value| s["appId"].as_i64().map(|n| n.to_string()).or_else(|| s["appId"].as_str().map(str::to_owned)) == Some(variant.clone());
        match r["sessions"].as_array().and_then(|a| a.iter().find(same)) {
            Some(s) => {
                let params = json!({"sessionId":s["sessionId"],"streamingBaseUrl":s["streamingBaseUrl"],"appId":variant,"recoveryMode":true});
                self.enqueue("session.claim",params,Purpose::Create,None);
            }
            None => self.rejected(Job { method:"session.create",params:Value::Null,purpose:Purpose::Create,seq:None,epoch:self.epoch },"session_conflict"),
        }
    }
    fn session_update(&mut self, r: &Value) {
        let value = if r["session"].is_object() { &r["session"] } else { r };
        let Some(id) = value["sessionId"].as_str() else { return; };
        let Some(l) = &mut self.launch else { return; };
        if l.cloud["sessionId"].as_str().is_some_and(|current| current != id) { return; }
        l.cloud = value.clone();
        if l.stopping { return; }
        let status = value["status"].as_i64().unwrap_or(0);
        let phase = value["phase"].as_str().unwrap_or("");
        if status == 7 || phase == "finished" || phase == "failed" {
            self.finish(SessionState::Ended { reason:EndReason::Remote });
            self.enqueue("streamer.stop",json!({"reason":"the remote session ended"}),Purpose::Pause,None);
        } else if matches!(status,2|3) && value["rtspsEndpoints"].as_array().is_some_and(|a| !a.is_empty()) && value["resumePending"] != true {
            if !l.started {
                l.started = true; l.poll_due = 0;
                self.session(SessionState::Starting);
                self.start();
            }
        } else {
            l.poll_due = self.now+1000;
            if phase == "queued" || value["queuePosition"].as_u64().is_some() || status == 1 {
                self.session(SessionState::Queue { position:value["queuePosition"].as_u64().and_then(|n| u32::try_from(n).ok()),eta_s:value["queueEtaSeconds"].as_u64().and_then(|n| u32::try_from(n).ok()) });
            } else { self.session(SessionState::Starting); }
        }
    }
    fn running(&mut self) {
        let Some(l) = &self.launch else { return; };
        let mut p = settings::profile(&l.values,&self.hello.display);
        let agreed = &l.cloud["negotiatedStreamProfile"];
        if let Some(codec) = agreed["codec"].as_str() { p.codec = match codec { "av1"=>Codec::Av1,"h264"=>Codec::H264,_=>Codec::H265 }; }
        if let Some(res) = agreed["resolution"].as_str().and_then(|s| s.split_once('x')) {
            if let (Ok(w),Ok(h)) = (res.0.parse(),res.1.parse()) { p.width=w; p.height=h; }
        }
        if let Some(fps) = agreed["fps"].as_u64().and_then(|n| u32::try_from(n).ok()) { p.fps=fps; }
        if let Some(hdr) = agreed["enableHdr"].as_bool() { p.hdr=hdr; }
        if settings::boolean(&l.values,"hdr") {
            p.sdr_reason = if l.cloud["hdrDowngrade"] == "game-unsupported" {Some("SDR: no HDR for this game".into())}
                else if agreed["enableHdr"] == false {Some("SDR: GeForce NOW sent no HDR".into())} else {None};
        }
        self.session(SessionState::Running { stream:Some(p) });
    }
    fn stats(&mut self, r: &Value) {
        let float = |key| r[key].as_f64().filter(|n| n.is_finite()).map(|n| n as f32);
        let Some(l) = self.launch.as_mut().filter(|l| l.running) else { return; };
        let trip = &mut l.trip;
        trip.network_ms = float("roundTripTimeMs").or(trip.network_ms);
        trip.host_ms = float("hostProcessingLatencyMs").or(trip.host_ms);
        trip.client_ms = float("decodeLatencyMs").or(trip.client_ms);
        if !l.stats { return; }
        let session = l.id;
        let bitrate_kbps = r["bitrateMbps"].as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| (n*1000.0).round() as u32);
        let pad = l.pad.as_ref().map(|(stats, at)| PadStats { age_ms: stats.age_ms.saturating_add(self.now.saturating_sub(*at)), ..stats.clone() });
        self.emit(None,Event::Stats { session,stats:Stats {
            fps:float("framesPerSecond"),bitrate_kbps,dropped:r["queueDropCount"].as_u64(),
            rtt_ms:float("roundTripTimeMs"),jitter_ms:float("jitterMs"),loss_pct:float("packetLossPercent"),
            client_ms:float("decodeLatencyMs"),host_ms:float("hostProcessingLatencyMs"),pad,
        } });
    }
    fn event(&mut self, name: &str, payload: &Value) {
        match name {
            "session.changed" if self.launch.as_ref().is_some_and(|l| l.cloud["sessionId"].is_string()) => self.session_update(payload),
            "streamer.changed" => {
                let Some(l) = &self.launch else { return; };
                if l.stopping || l.retry_stopping { return; }
                let status = payload["status"].as_str().unwrap_or("");
                if l.running && matches!(status,"error"|"stopped"|"exited") {
                    let state = if status == "error" {SessionState::Failed { reason:FailReason::Video,message:"The stream stopped unexpectedly".into() }} else {SessionState::Ended { reason:EndReason::Remote }};
                    self.finish(state);
                    self.enqueue("streamer.stop",json!({"reason":"the stream ended"}),Purpose::Pause,None);
                } else { self.stats(payload); }
            }
            "auth.session.changed" => {
                if payload["generation"].as_u64() < self.scope["generation"].as_u64() { return; }
                if payload["session"].is_null() {
                    self.epoch += 1;
                    self.queue.clear(); self.cursors.clear(); self.region=None;
                    if self.launch.is_some() { self.finish(SessionState::Failed { reason:FailReason::Auth,message:"GeForce NOW signed out".into() }); self.enqueue("streamer.stop",json!({"reason":"the account signed out"}),Purpose::Pause,None); }
                    self.target=None; self.scope=Value::Null; self.selections.clear(); self.targets(None);
                } else { self.account(payload,None,false); }
            }
            _ => {}
        }
    }
}

pub fn descriptor() -> Descriptor {
    Descriptor { id:EngineId::new("gfn").unwrap(),version:env!("CARGO_PKG_VERSION").into(),capabilities:Capabilities {
        kind:EngineKind::Cloud,targets:TargetKind::Account,auth:vec![AuthMethod::DeviceCode],presentation:Presentation::Feeder,
        features:BTreeSet::from([Feature::Readiness,Feature::Queue,Feature::EndGame,Feature::Stats,Feature::RoundTrip,Feature::Rumble,Feature::OverlayKeys]),
        video:Some(VideoCaps { codecs:vec![Codec::H265,Codec::Av1,Codec::H264],width:3840,height:2160,fps:120,hdr10:true }),
    } }
}
