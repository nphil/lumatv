//! The compositor's luna methods (tier 1 of `docs/research/compositor-c2-result.md` §5) and the tvpower
//! handshake, as pure state: an [`Inbound`] in, [`Reply`]s out, pushes to subscribers when the scene changes.
//! Payload shapes are the ones X4 captured from LG's compositor (`docs/research/arch/x4/`).

use serde_json::{Value, json};
use wayland_server::Resource;

use crate::State;
use crate::luna::{Event, Inbound, Outbound, Reply, ToBridge, power};
use crate::window::WindowType;

pub struct Subscription {
    pub token: u64,
    pub service: String,
    pub method: String,
    pub sender: String,
    /// The last payload pushed, so an unchanged state is not pushed again.
    pub last: Value,
}

pub struct LunaState {
    pub subscriptions: Vec<Subscription>,
    pub screen_visible: bool,
    pub key_block: bool,
    pub pin_overlay: Option<Value>,
    pub power: Power,
    next_token: u64,
    /// Outbound tokens of the tvpower registrations, to tell their pushes apart.
    power_tokens: Vec<(u64, &'static str)>,
    nooperation: crate::nooperation::Timers,
    /// The audio service's volume subscription, and the mute state it last pushed (what Mute toggles from).
    audio_token: Option<u64>,
    /// The way out of LG's saver (`saverguard`): the wake call in flight, its escalation.
    pub(crate) saver: crate::saverguard::SaverGuard,
    /// The standing subscriptions kept alive (`subwatch`).
    pub(crate) watches: Vec<crate::subwatch::Watch>,
    pub muted: bool,
    /// The tokens of the video calls (placements, properties, mute) that have not been answered yet: (token, method, context). Capped: an answer
    /// that never comes must not grow it.
    video_calls: Vec<(u64, String, String)>,
    cover: CoverLaunch,
    /// LG's guide: the life event subscription and the closes asked of SAM (`lg_guide.rs`).
    pub(crate) guide: crate::lg_guide::Answer,
}


struct CoverLaunch {
    pending: [Value; 4],
    token: Option<u64>,
    status: &'static str,
}
impl Default for CoverLaunch {
    fn default() -> Self { Self { pending: Default::default(), token: None, status: "closed" } }
}

impl CoverLaunch {
    fn prepare(&mut self, payload: &Value) -> bool {
        let input_params = match payload.get("inputParams") {
            Some(p) if p.is_object() => Some(p),
            Some(_) => return false,
            None => None,
        };
        let mut pending = [const { Value::Null }; 4];
        for (index, key) in ["firstApp", "secondApp", "fallback", "finalFallback"].into_iter().enumerate() {
            if let Some(app) = payload.get(key) {
                if app.is_null() { continue; }
                let Some(obj) = app.as_object() else { return false; };
                let Some(id) = obj.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else { continue; };
                if obj.get("params").is_some_and(|p| !p.is_object()) { return false; }
                let mut app = app.clone();
                let input = matches!(id, "com.webos.app.livetv" | "com.webos.app.hdmi1" | "com.webos.app.hdmi2"
                    | "com.webos.app.hdmi3" | "com.webos.app.hdmi4" | "com.webos.app.dp1" | "com.webos.app.dp2"
                    | "com.webos.app.usbc1" | "com.webos.app.usbc2" | "com.webos.app.externalinput.av1"
                    | "com.webos.app.externalinput.av2" | "com.webos.app.externalinput.component"
                    | "com.webos.app.externalinput.scart" | "com.webos.app.smhl");
                if !obj.contains_key("params") {
                    app["params"] = if input { input_params.cloned().unwrap_or_else(|| json!({"boot": true})) } else { json!({}) };
                }
                pending[index] = app;
            }
        }
        if pending.iter().all(Value::is_null) { return false; }
        self.pending = pending;
        self.token = None;
        self.status = "launching";
        true
    }
}

impl LunaState {
    pub(crate) fn note_video_call(&mut self, token: u64, method: String, context: String) {
        if self.video_calls.len() >= 32 {
            self.video_calls.remove(0);
        }
        self.video_calls.push((token, method, context));
    }

    /// The journal line for the answer to a video call, when `token` is one: {"returnValue": false, ...} is the case that matters, but every
    /// answer is kept: the line is how a placement is known to have been accepted.
    pub(crate) fn video_answer(&mut self, token: u64, payload: &Value) -> Option<String> {
        let at = self.video_calls.iter().position(|(t, ..)| *t == token)?;
        let (_, method, context) = self.video_calls.remove(at);
        Some(format!("videooutput: {method} for {context:?} answered {payload}"))
    }
}

impl Default for LunaState {
    fn default() -> Self {
        LunaState {
            subscriptions: Vec::new(),
            screen_visible: true,
            key_block: false,
            pin_overlay: None,
            power: Power::default(),
            next_token: 0,
            power_tokens: Vec::new(),
            nooperation: crate::nooperation::Timers::default(),
            audio_token: None,
            saver: crate::saverguard::SaverGuard::default(),
            watches: Vec::new(),
            cover: CoverLaunch::default(),
            video_calls: Vec::new(),
            guide: crate::lg_guide::Answer::default(),
            muted: false,
        }
    }
}

impl Default for Power {
    fn default() -> Self {
        Power { state: "unknown".into(), sub_state: String::new(), rendering: true, pending: Vec::new() }
    }
}

/// tvpower's rounds as the compositor sees them (`StarfishPowerService.qml`): a push with `timestamp` and
/// `next sub state` on one of the three registrations, answered with the matching `response*` carrying the
/// same timestamp and `ack: true`.
pub struct Power {
    pub state: String,
    /// `sub state` of the same push: `screen saver` while LG's saver is up with `state` still `ACTIVE` (`StarfishScreenSaverView.qml`).
    pub sub_state: String,
    /// False between "prepare power off" and "prepare power on": nothing is drawn.
    pub rendering: bool,
    /// Rounds answered, for the journal: (round, timestamp, next sub state).
    pub pending: Vec<(String, Value, String)>,
}

impl Power {
    /// Handles a push on registration `round` ("PreparePowerOff", "PreparePowerOn", "PowerOffLogoRequest");
    /// returns the response call to make, if the push was a real round.
    pub fn on_push(&mut self, round: &str, payload: &Value) -> Option<(String, Value)> {
        let ts = payload.get("timestamp")?;
        let next = payload.get("next sub state").and_then(Value::as_str).unwrap_or("").to_string();
        match round {
            "PreparePowerOff" => {
                if matches!(next.as_str(), "active standby" | "suspend" | "always ready" | "reboot" | "cold") {
                    self.rendering = false;
                } else {
                    log::warn!("tvpower: prepare power off with next sub state {next:?}");
                }
            }
            "PreparePowerOn" => {
                if matches!(next.as_str(), "active" | "always on display") {
                    self.rendering = true;
                } else {
                    log::warn!("tvpower: prepare power on with next sub state {next:?}");
                }
            }
            "PowerOffLogoRequest" => {}
            _ => return None,
        }
        if self.pending.len() >= 16 { self.pending.remove(0); }
        self.pending.push((round.to_string(), ts.clone(), next));
        Some(power::response(round, ts, true))
    }

    pub fn on_state(&mut self, payload: &Value) {
        if let Some(s) = payload.get("state").and_then(Value::as_str) {
            self.state = s.to_string();
        }
        if let Some(s) = payload.get("sub state").and_then(Value::as_str) {
            self.sub_state = s.to_string();
        }
    }

    /// LG's saver is the set's state (its window may not be mapped yet).
    pub fn saver(&self) -> bool {
        self.sub_state.eq_ignore_ascii_case("screen saver") || self.state.eq_ignore_ascii_case("screen saver")
    }

    /// What a press of Power asks of tvpower: standby from `ACTIVE` and from LG's saver, the screen on from `POWER OFF`, nothing in
    /// a transition (`request power on`, ...) or before the first state is known.
    pub fn key_request(&self) -> Option<&'static str> {
        if self.state.eq_ignore_ascii_case("active") || self.saver() {
            Some("powerOff")
        } else if self.state.eq_ignore_ascii_case("power off") {
            Some("powerOn")
        } else {
            None
        }
    }
}

const HOME_UI_TYPES: [&str; 7] = ["toast", "inputAlert", "alert", "pincodePrompt", "quickSetting", "com.webos.app.voiceagent", "com.webos.app.touchremote"];

const AUDIO: &str = "luna://com.webos.service.audio";
/// LG's saver app (a Flutter client, `com.webos.app.screensaver`).
const LG_SAVER_APP: &str = "com.webos.app.screensaver";

/// `ctl status`'s `power`: tvpower's state, LG's idle timers, the standing subscriptions, and the way out of LG's saver.
impl State {
    pub fn luna_report(&self) -> Value {
        json!({
            "state": self.luna_state.power.state, "rendering": self.luna_state.power.rendering,
            "lgSaver": {"window": self.lg_saver_up(), "present": self.lg_saver_present(), "guard": self.luna_state.saver.report()},
            "idleTimers": self.luna_state.nooperation.report(std::time::Instant::now()),
            "subscriptions": self.luna_state.watches.iter().map(crate::subwatch::Watch::report).collect::<Vec<_>>(),
        })
    }
}

impl State {
    pub(crate) fn luna_token(&mut self) -> u64 {
        self.luna_state.next_token += 1;
        self.luna_state.next_token
    }

    fn cover_next(&mut self) {
        let Some(app) = self.luna_state.cover.pending.iter_mut().find_map(|app| (!app.is_null()).then(|| app.take())) else {
            self.luna_state.cover.status = "failed";
            self.journal("coverUI: every launch candidate failed".into());
            return;
        };
        const LAUNCH: &str = "luna://com.webos.applicationManager/launch";
        if self.remote_service_refused(LAUNCH, &app) {
            self.journal("luna: LG Remote Service is blocked (Settings > Privacy), launch dropped".into());
            self.cover_next();
            return;
        }
        let token = self.luna_token();
        self.luna_state.cover.token = Some(token);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: LAUNCH.into(), payload: app, subscribe: false }));
    }

    fn cover_answer(&mut self, token: u64, payload: &Value) -> bool {
        if self.luna_state.cover.token != Some(token) { return false; }
        self.luna_state.cover.token = None;
        if payload.get("returnValue").and_then(Value::as_bool) == Some(true) {
            self.luna_state.cover.pending.fill(Value::Null);
            self.luna_state.cover.status = "closed";
            self.luna_call("luna://com.webos.bootManager/generateSignal".into(), json!({"name": "firstapp-launched"}));
        } else if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
            self.cover_next();
        } else {
            self.luna_state.cover.pending.fill(Value::Null);
            self.luna_state.cover.status = "failed";
            self.journal("coverUI: invalid launch response".into());
        }
        true
    }

    /// Registers with tvpower and subscribes to the audio service's volume state (call once the bridge is up).
    pub fn luna_register(&mut self) {
        self.luna_state.saver = crate::saverguard::SaverGuard::default();
        self.luna_state.watches.clear();
        self.luna_state.power_tokens.clear();
        let now = std::time::Instant::now();
        let calls = power::register_calls();
        let rounds = ["PreparePowerOff", "PreparePowerOn", "PowerOffLogoRequest", "PowerState"];
        for ((uri, payload), round) in calls.into_iter().zip(rounds) {
            let token = self.luna_token();
            self.luna_state.power_tokens.push((token, round));
            self.luna_state.watches.push(crate::subwatch::Watch::new(crate::subwatch::What::Power(round), uri.clone(), payload.clone(), token, now));
            self.luna.send(ToBridge::Outbound(Outbound { token, uri, payload, subscribe: true }));
        }
        let token = self.luna_token();
        self.luna_state.audio_token = Some(token);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: format!("{AUDIO}/master/getVolume"), payload: json!({"subscribe": true}), subscribe: true }));
        // the hardware video windows read the video sinks' status while their switch is on (`vwin`)
        self.vwin_subscribe();
        let token = self.luna_token();
        self.luna_state.nooperation.token = Some(token);
        let payload = json!({"subscribe": true});
        self.luna_state.watches.push(crate::subwatch::Watch::new(crate::subwatch::What::Timers, crate::nooperation::TIMER_INFO.into(), payload.clone(), token, now));
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: crate::nooperation::TIMER_INFO.into(), payload, subscribe: true }));
        self.lg_guide_subscribe();
    }

    /// Makes again each subscription that was refused, never answered, or is due its refresh (`subwatch`).
    fn watches_tick(&mut self, now: std::time::Instant) {
        for i in 0..self.luna_state.watches.len() {
            if !self.luna_state.watches[i].due(now) { continue; }
            let old = self.luna_state.watches[i].token;
            let token = self.luna_token();
            let w = &mut self.luna_state.watches[i];
            let (what, uri, payload) = (w.what, w.uri.clone(), w.payload.clone());
            w.restart(token, now);
            let restarts = w.restarts;
            match what {
                crate::subwatch::What::Power(round) => {
                    if let Some(entry) = self.luna_state.power_tokens.iter_mut().find(|(t, _)| *t == old) { *entry = (token, round); }
                }
                crate::subwatch::What::Timers => self.luna_state.nooperation.token = Some(token),
            }
            self.luna.send(ToBridge::Cancel { token: old });
            if restarts <= 3 || restarts % 20 == 0 {
                self.journal(format!("luna: {uri} subscribed again (try {restarts})"));
            }
            self.luna.send(ToBridge::Outbound(Outbound { token, uri, payload, subscribe: true }));
        }
    }

    /// An answer to one of the compositor's own calls: tvpower pushes and the audio service's volume state are the
    /// ones acted on; the shell's calls are answered to it.
    pub fn luna_event(&mut self, ev: Event) {
        let now = std::time::Instant::now();
        for w in &mut self.luna_state.watches {
            if w.note(ev.token, &ev.payload, now) { break; }
        }
        let ok = ev.payload.get("returnValue").and_then(Value::as_bool) != Some(false);
        if self.luna_state.saver.answered(ev.token, ok) {
            if !ok {
                self.journal(format!("screensaver: LG wake refused: {}", ev.payload));
            }
            return;
        }
        if self.luna_state.nooperation.token == Some(ev.token) {
            self.luna_state.nooperation.update(&ev.payload, std::time::Instant::now());
            return;
        }
        if self.cover_answer(ev.token, &ev.payload) { return; }
        if self.lg_guide_event(ev.token, &ev.payload) {
            return;
        }
        if self.panel_answer(ev.token, &ev.payload) {
            return;
        }
        if let Some(line) = self.luna_state.video_answer(ev.token, &ev.payload) {
            self.journal(line);
            return;
        }
        if Some(ev.token) == self.vwin.status_token {
            self.vwin_status(&ev.payload);
            return;
        }
        if Some(ev.token) == self.luna_state.audio_token {
            // X4: {"volumeStatus": {"muteStatus": false, "soundOutput": "external_arc", "volume": 0, ...}}
            if let Some(m) = ev.payload.pointer("/volumeStatus/muteStatus").and_then(Value::as_bool) {
                self.luna_state.muted = m;
            }
            self.shell_audio(&ev.payload);
            return;
        }
        if self.shell_luna_event(ev.token, &ev.payload) {
            return;
        }
        let Some(round) = self.luna_state.power_tokens.iter().find(|(t, _)| *t == ev.token).map(|(_, r)| *r) else {
            log::debug!("luna event token {}: {}", ev.token, ev.payload);
            return;
        };
        if round == "PowerState" {
            // The standby's timeline: each state tvpower goes through, from the Power key (the rounds below are its asks of the compositor).
            let was = (self.luna_state.power.state.clone(), self.luna_state.power.sub_state.clone());
            self.luna_state.power.on_state(&ev.payload);
            let now = (&self.luna_state.power.state, &self.luna_state.power.sub_state);
            if (now.0, now.1) != (&was.0, &was.1) {
                let line = format!("tvpower: the set is {:?}/{:?} (was {:?}/{:?}){}", now.0, now.1, was.0, was.1, self.since_power_key());
                self.journal(line);
            }
            return;
        }
        if let Some((uri, payload)) = self.luna_state.power.on_push(round, &ev.payload) {
            if round == "PreparePowerOn" && self.luna_state.power.rendering {
                self.nooperation_activity();
            }
            self.refresh_policy_sync();
            let next = ev.payload.get("next sub state").and_then(Value::as_str).unwrap_or("");
            let line = format!("tvpower: {round} ({next}) -> ack{}", self.since_power_key());
            self.journal(line);
            // the user's panel brightness goes back before the answer that lets the TV power down (`paneldim`)
            if !self.luna_state.power.rendering {
                self.panel_standby();
            }
            let token = self.luna_token();
            self.luna.send(ToBridge::Outbound(Outbound { token, uri, payload, subscribe: false }));
            self.shell_sync_topics();
        }
    }

    pub fn nooperation_deadline(&self) -> Option<std::time::Instant> {
        self.luna_state.power.rendering.then(|| self.luna_state.nooperation.deadline()).flatten()
    }

    /// LG's saver is there: its window is mapped above the app, or tvpower's sub state says `screen saver` (its window may not be mapped yet;
    /// `state` stays `ACTIVE` through it).
    pub fn lg_saver_present(&self) -> bool {
        self.lg_saver_up() || self.luna_state.power.saver()
    }

    pub fn lg_saver_up(&self) -> bool {
        self.scene.windows().any(|w| w.kind == WindowType::Screensaver
            && w.state != crate::window::ShellState::Minimized && w.map_serial > 0
            && self.surfaces.get(w.surface).is_some_and(|s| s.is_mapped()))
    }

    /// LG's saver has the keys: it is there and has not been given up on (`saverguard`).
    pub fn lg_saver_engaged(&self) -> bool {
        self.luna_state.saver.holds_keys(self.lg_saver_present())
    }

    /// A key or the pointer while LG's saver may be up: ask tvpower to end it, and when that does not, close its app and then stop
    /// swallowing the keys and hide its window (`saverguard`).
    pub(crate) fn wake_lg_saver(&mut self) {
        self.wake_lg_saver_at(std::time::Instant::now());
    }

    pub fn wake_lg_saver_at(&mut self, now: std::time::Instant) {
        let step = self.luna_state.saver.input(self.lg_saver_present(), now);
        self.apply_saver_step(step, now);
    }

    fn apply_saver_step(&mut self, step: crate::saverguard::Step, now: std::time::Instant) {
        if step.wake {
            let token = self.luna_token();
            self.luna_state.saver.sent(token, now);
            self.luna.send(ToBridge::Outbound(Outbound { token,
                uri: "luna://com.webos.service.tvpower/power2/turnOnScreen".into(),
                payload: json!({}), subscribe: false }));
        }
        if step.close {
            self.journal("screensaver: still up 3 s after a key; its app is asked to close".into());
            self.close_app(LG_SAVER_APP);
            self.luna_call("luna://com.webos.applicationManager/closeByAppId".into(), json!({"id": LG_SAVER_APP, "reason": "windowPolicy"}));
        }
        if step.abandon {
            let ids: Vec<_> = self.scene.windows().filter(|w| w.kind == WindowType::Screensaver).map(|w| w.id).collect();
            for id in ids {
                if let Some(w) = self.scene.window_mut(id) { w.state = crate::window::ShellState::Minimized; }
            }
            crate::server::webos_shell::sync_exposed(self);
            crate::server::seat::refocus(self);
            self.luna_notify();
            self.journal("screensaver: still up 6 s after a key; its window is hidden and the keys go to the app under it".into());
        }
    }

    pub(crate) fn nooperation_activity(&mut self) {
        self.luna_state.nooperation.activity(std::time::Instant::now());
    }

    /// Runs even when no frames are drawn: the standing subscriptions kept alive, the end of a saver episode seen, and LG's idle timers
    /// reported. LG still decides whether media permits the saver.
    pub fn nooperation_tick(&mut self) {
        self.nooperation_tick_at(std::time::Instant::now());
    }

    pub fn nooperation_tick_at(&mut self, now: std::time::Instant) {
        self.watches_tick(now);
        let present = self.lg_saver_present();
        if self.luna_state.power.rendering {
            let step = self.luna_state.saver.tick(present, now);
            self.apply_saver_step(step, now);
        } else {
            self.luna_state.saver.observe(present);
            return;
        }
        while let Some(kind) = self.luna_state.nooperation.expired(now) {
            let payload = json!({"type": kind, "status": "expired"});
            self.luna_call(crate::nooperation::TIMER_EXPIRED.into(), payload);
        }
    }

    /// Volume and Mute the way LG's key filters and its audio plugin (`AudioService/libaudioserviceplugin.so`, the
    /// URIs and the `{}`/`{"mute":…}` payloads are its strings) do them: up or down on every press and key repeat,
    /// `volumeKeeping` on release, Mute toggled on release from the state the audio service last pushed. With the
    /// receiver on ARC the service passes the steps on over CEC (PLAN: Receiver).
    pub fn system_key(&mut self, code: u32, pressed: bool) {
        use crate::input::code::{KEY_MUTE, KEY_VOLUMEDOWN, KEY_VOLUMEUP};
        let (method, payload) = match (code, pressed) {
            (KEY_VOLUMEUP, true) => ("master/volumeUp", json!({})),
            (KEY_VOLUMEDOWN, true) => ("master/volumeDown", json!({})),
            (KEY_VOLUMEUP | KEY_VOLUMEDOWN, false) => ("volumeKeeping", json!({})),
            (KEY_MUTE, false) => {
                self.luna_state.muted = !self.luna_state.muted;
                ("master/muteVolume", json!({"mute": self.luna_state.muted}))
            }
            _ => return,
        };
        let token = self.luna_token();
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: format!("{AUDIO}/{method}"), payload, subscribe: false }));
        self.keys.delivered += 1;
    }

    /// A call to one of the compositor's bus names.
    pub fn luna_inbound(&mut self, m: Inbound) {
        if m.cancelled {
            if let Some(s) = self.luna_state.subscriptions.iter().find(|s| s.token == m.token) {
                let line = format!("luna: {} cancelled {}/{}", s.sender, s.service, s.method);
                self.journal(line);
            }
            self.luna_state.subscriptions.retain(|s| s.token != m.token);
            return;
        }
        let payload = match self.luna_method(&m.service, &m.method, &m.payload, &m.sender) {
            Some(mut p) => {
                if m.subscribe
                    && let Some(obj) = p.as_object_mut()
                {
                    // `last` is kept without the flag: pushes carry none (LG's do not), so the comparison in
                    // `luna_notify` sees only a real change.
                    let last = Value::Object(obj.clone());
                    obj.insert("subscribed".into(), json!(true));
                    self.luna_state.subscriptions.push(Subscription {
                        token: m.token,
                        service: m.service.clone(),
                        method: m.method.clone(),
                        sender: m.sender.clone(),
                        last,
                    });
                    self.journal(format!("luna: {} subscribed to {}/{}", m.sender, m.service, m.method));
                }
                p
            }
            None => json!({"returnValue": false, "errorCode": -1, "errorText": format!("unknown method {}/{}", m.service, m.method)}),
        };
        self.luna.send(ToBridge::Reply(Reply { token: m.token, payload }));
    }

    /// Pushes every subscription whose payload changed. Called after the scene or input state changed. A method is
    /// computed once however many subscribe to it (activitymanager holds 14 `getForegroundAppInfo` subscriptions on
    /// the set), the replies go to the bridge's queue, and one journal line covers a foreground push to all of them.
    /// The shell's topics follow the same changes.
    pub fn luna_notify(&mut self) {
        self.wall_handover();
        self.shell_sync_topics();
        if self.luna_state.subscriptions.is_empty() {
            return;
        }
        let mut subs = std::mem::take(&mut self.luna_state.subscriptions);
        // (first subscription with this service and method, its payload)
        let mut computed: Vec<(usize, Option<Value>)> = Vec::new();
        let mut pushed = vec![false; subs.len()];
        for i in 0..subs.len() {
            let k = match computed.iter().position(|(j, _)| subs[*j].service == subs[i].service && subs[*j].method == subs[i].method) {
                Some(k) => k,
                None => {
                    let p = self.luna_method(&subs[i].service, &subs[i].method, &json!({}), &subs[i].sender);
                    computed.push((i, p));
                    computed.len() - 1
                }
            };
            let Some(p) = &computed[k].1 else { continue };
            let sub = &mut subs[i];
            if sub.last != *p {
                sub.last = p.clone();
                self.luna.send(ToBridge::Reply(Reply { token: sub.token, payload: p.clone() }));
                pushed[i] = true;
            }
        }
        for (j, p) in &computed {
            let Some(p) = p else { continue };
            if subs[*j].method != "getForegroundAppInfo" {
                continue;
            }
            let to: Vec<&Subscription> = (0..subs.len())
                .filter(|i| pushed[*i] && subs[*i].service == subs[*j].service && subs[*i].method == subs[*j].method)
                .map(|i| &subs[i])
                .collect();
            if to.is_empty() {
                continue;
            }
            let apps: Vec<&str> = p["foregroundAppInfo"].as_array().into_iter().flatten().filter_map(|a| a["appId"].as_str()).collect();
            let mut senders: Vec<&str> = to.iter().map(|s| s.sender.as_str()).collect();
            senders.dedup();
            let line = format!("luna: foreground {apps:?} pushed to {} subscriber(s) ({})", to.len(), senders.join(", "));
            self.journal(line);
        }
        self.luna_state.subscriptions = subs;
    }

    /// The method table. `None` for a method this compositor does not have.
    pub fn luna_method(&mut self, service: &str, method: &str, payload: &Value, _sender: &str) -> Option<Value> {
        let ok = || json!({"returnValue": true});
        Some(match (service, method) {
            ("com.webos.surfacemanager", "getForegroundAppInfo") | ("com.webos.service.systemUIManager", "getForegroundAppInfo") => {
                self.foreground_app_info(service == "com.webos.service.systemUIManager")
            }
            ("com.webos.surfacemanager", "getFocusedWindowInfo") => self.focused_window_info(),
            ("com.webos.surfacemanager", "getCurrentState") | ("com.webos.surfacemanager", "getForegroundWindowInfo") => self.window_list(),
            ("com.webos.surfacemanager", "getForegroundVideoWindow") => json!({"returnValue": true, "foregroundVideoWindow": self.video_windows()}),
            ("com.webos.surfacemanager", "getLastInputType") => {
                json!({"returnValue": true, "lastInputType": if self.seat.last_input.is_empty() { "key" } else { self.seat.last_input }})
            }
            ("com.webos.surfacemanager", "closeByAppId") | ("com.webos.service.systemUIManager", "closeByAppId") => {
                let Some(app) = payload.get("appId").and_then(Value::as_str) else {
                    return Some(json!({"returnValue": false, "errorText": "appId required"}));
                };
                let n = self.close_app(app);
                json!({"returnValue": true, "appId": app, "closed": n})
            }
            ("com.webos.surfacemanager", "setLaunchOrigin") => self.set_launch_origin(payload),
            ("com.webos.surfacemanager", "getScreenVisible") => json!({"returnValue": true, "screenVisible": self.luna_state.screen_visible}),
            ("com.webos.surfacemanager", "setScreenVisible") => {
                self.luna_state.screen_visible = payload.get("screenVisible").and_then(Value::as_bool).unwrap_or(true);
                ok()
            }
            ("com.webos.surfacemanager", "setPinOverlayView") => {
                self.luna_state.pin_overlay = Some(payload.clone());
                self.journal(format!("luna: setPinOverlayView {payload}"));
                ok()
            }
            ("com.webos.surfacemanager", "getRecentsAppList") | ("com.webos.service.recent", "listRecents") => {
                json!({"returnValue": true, "recentsAppList": self.recents(), "recents": self.recents()})
            }
            ("com.webos.surfacemanager", "clearRecentsList") | ("com.webos.service.recent", "removeAllRecents") => ok(),
            ("com.webos.service.recent", "removeRecent") => json!({"returnValue": true, "appId": payload.get("appId").cloned().unwrap_or(Value::Null)}),
            ("com.webos.surfacemanager", "showInputPicker") => {
                if self.shell_show("picker", false) {
                    self.journal(format!("luna: {method} -> LumaTV's input picker"));
                } else {
                    self.journal(format!("luna: {method} {payload} (the shell cannot show it now)"));
                }
                ok()
            }
            ("com.webos.surfacemanager", "showQuickSettings") => {
                if self.shell_show("quick", false) {
                    self.journal(format!("luna: {method} -> LumaTV's quick panel"));
                } else {
                    self.journal(format!("luna: {method} {payload} (the shell cannot show it now)"));
                }
                ok()
            }
            ("com.webos.service.systemUIManager", "launch") => {
                // LG's key filters and LumaTV's own QML open the system windows this way; the two the shell draws are its.
                let kind = match payload.get("appId").and_then(Value::as_str) {
                    Some("com.webos.app.quicksettings") => Some("quick"),
                    Some("com.webos.app.quickinputpicker") => Some("picker"),
                    _ => None,
                };
                if let Some(k) = kind
                    && self.shell_show(k, false)
                {
                    self.journal(format!("luna: systemUIManager/launch {} -> LumaTV's {k}", payload.get("appId").and_then(Value::as_str).unwrap_or("")));
                } else {
                    self.journal(format!("luna: {method} {payload} (no window of its own here)"));
                }
                ok()
            }
            ("com.webos.surfacemanager.home", "getCurrentHomeUIState") => self.home_ui_state(),
            ("com.webos.surfacemanager.input", "getKeyInputBlockState") => json!({"returnValue": true, "blockState": self.luna_state.key_block}),
            ("com.webos.surfacemanager.input", "blockKeyInput") => {
                self.luna_state.key_block = true;
                ok()
            }
            ("com.webos.surfacemanager.input", "cancelBlockKeyInput") => {
                self.luna_state.key_block = false;
                ok()
            }
            ("com.webos.surfacemanager.input", "getForegroundAppKeymask") => {
                let fg = self.scene.foreground(&self.surfaces);
                json!({"returnValue": true, "appId": fg.map(|w| w.app_id().to_string()).unwrap_or_default(), "keyMask": fg.map(|w| w.key_mask).unwrap_or(0)})
            }
            ("com.webos.coverUI", "getStatus") => json!({"returnValue": true, "status": self.luna_state.cover.status}),
            ("com.webos.coverUI", "open") => {
                if self.luna_state.cover.prepare(payload) {
                    self.cover_next();
                    ok()
                } else {
                    json!({"returnValue": false, "errorText": "valid firstApp, secondApp or fallback required"})
                }
            }
            ("com.webos.coverUI", "close") => {
                self.luna_state.cover.pending.fill(Value::Null);
                self.luna_state.cover.token = None;
                self.luna_state.cover.status = "closed";
                ok()
            }
            ("com.webos.surfacemanager.systemui", "closeAllSystemUI") => ok(),
            ("com.webos.surfacemanager.screenSaver", "restartUserActivityTimer") => ok(),
            _ => return None,
        })
    }

    /// LG's `StarfishForegroundAppInfoMgr.qml` (payloads as X4 captured them): the foreground card first, then every
    /// mapped app window above it (Settings came as an overlay). A window that owns a surface group is listed with it as LG's
    /// `StarfishUtils.foregroundList` has it, the members below the owner, the owner, the members above it; a member is no item
    /// of its own (it is no card or overlay: `Window::in_group`). Items without an appId are left out; CARD and
    /// RESTRICTED windows that are not members of another window's group carry `multiview`, `pip`, `primary`,
    /// `viewType` and `order`; surface-group owners and members say so (`windowGroupOwner`, `windowGroupOwnerId`).
    /// SAM turns these pushes into its own `foreground`/`background` events and keeps group members out of its fullscreen
    /// foreground apps: given Live Dmost alone (it had come to the front as a card) it had no foreground app and launched Home a
    /// second later, and videooutputd disconnected the video ("main appId Error"; the set's SAM and videooutputd logs, 2026-10-28).
    /// The system UI manager's variant lists the system UI windows.
    fn foreground_app_info(&self, system_ui: bool) -> Value {
        let mapped = |w: &&crate::window::Window| w.map_serial > 0 && self.surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false);
        let mut items: Vec<&crate::window::Window> = Vec::new();
        if system_ui {
            items.extend(self.scene.windows().filter(mapped).filter(|w| w.kind == WindowType::SystemUi));
        } else {
            let mut tops: Vec<&crate::window::Window> = Vec::new();
            tops.extend(self.scene.foreground(&self.surfaces));
            let mut above: Vec<&crate::window::Window> = self
                .scene
                .windows()
                .filter(mapped)
                .filter(|w| !w.in_group)
                .filter(|w| matches!(w.kind, WindowType::Overlay | WindowType::Floating | WindowType::Popup | WindowType::Screensaver) && w.state != crate::window::ShellState::Minimized)
                .collect();
            above.sort_by_key(|w| w.map_serial);
            tops.extend(above);
            for w in tops {
                self.push_with_group(w, &mut items);
            }
        }
        let infos: Vec<Value> = items.into_iter().filter(|w| !w.app_id().is_empty()).map(|w| self.foreground_item(w)).collect();
        json!({"returnValue": true, "mode": "normal", "foregroundAppInfo": infos})
    }

    /// `w` with the mapped windows attached to the group it owns: those below it (negative z) first, those above it after. A member
    /// that has no window of its own (a bare surface) has no app to list.
    fn push_with_group<'a>(&'a self, w: &'a crate::window::Window, out: &mut Vec<&'a crate::window::Window>) {
        let members = self.scene.group_members(w.surface);
        let window = |s: crate::surface::SurfaceId| {
            self.scene
                .window_of(s)
                .and_then(|id| self.scene.window(id))
                .filter(|m| m.map_serial > 0 && self.surfaces.get(m.surface).is_some_and(|s| s.is_mapped()))
        };
        out.extend(members.iter().filter(|(z, _)| *z < 0).filter_map(|(_, s)| window(*s)));
        out.push(w);
        out.extend(members.iter().filter(|(z, _)| *z >= 0).filter_map(|(_, s)| window(*s)));
    }

    fn foreground_item(&self, w: &crate::window::Window) -> Value {
        let owner = self.scene.groups.iter().any(|g| g.owner == w.surface);
        let owner_of_group = self.scene.group_of_member(w.surface);
        let mut item = json!({
            "appId": w.app_id(), "displayId": 0, "processId": self.window_pid(w), "windowType": self.window_type_name(w),
            "windowGroup": false, "params": self.window_params(w),
        });
        let obj = item.as_object_mut().unwrap();
        if owner || owner_of_group.is_some() {
            obj.insert("windowGroup".into(), json!(true));
            obj.insert("windowGroupOwner".into(), json!(owner));
            let owner_app = owner_of_group.and_then(|g| self.scene.window_of(g.owner)).and_then(|id| self.scene.window(id)).map(|o| o.app_id().to_string());
            obj.insert("windowGroupOwnerId".into(), json!(owner_app.unwrap_or_default()));
        }
        let fullscreen_type = matches!(self.window_type_name(w).as_str(), "_WEBOS_WINDOW_TYPE_CARD" | "_WEBOS_WINDOW_TYPE_RESTRICTED");
        if fullscreen_type && owner_of_group.is_none() {
            for (k, v) in [("multiview", json!(false)), ("pip", json!(false)), ("primary", json!(true)), ("viewType", json!("normal")), ("order", json!(0))] {
                obj.insert(k.into(), v);
            }
        }
        item
    }

    /// The window's client process id as LG sends it (a string); "" while the client's credentials are unknown.
    fn window_pid(&self, w: &crate::window::Window) -> String {
        let pid = self.seat.surfaces.get(&w.surface).and_then(|r| r.client()).and_then(|c| self.clients.get(&c.id())).map(|c| c.pid).unwrap_or(0);
        if pid == 0 { String::new() } else { pid.to_string() }
    }

    fn window_type_name(&self, w: &crate::window::Window) -> String {
        w.props.get("_WEBOS_WINDOW_TYPE").cloned().unwrap_or_else(|| "_WEBOS_WINDOW_TYPE_CARD".into())
    }

    fn window_params(&self, w: &crate::window::Window) -> Value {
        serde_json::from_str::<Value>(w.props.get("params").map(String::as_str).unwrap_or("{}")).unwrap_or(json!({}))
    }

    fn focused_window_info(&self) -> Value {
        let focus = self.seat.focus.and_then(|s| self.scene.window_of(s)).and_then(|w| self.scene.window(w));
        let list: Vec<Value> =
            focus.map(|w| json!({"appId": w.app_id(), "type": if w.kind.is_card() { "fullscreen" } else { "overlay" }})).into_iter().collect();
        json!({"returnValue": true, "focusedWindowInfo": list})
    }

    fn window_list(&self) -> Value {
        let list: Vec<Value> = self
            .scene
            .windows()
            .filter(|w| !w.in_group && self.surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
            .map(|w| json!({"appId": w.app_id(), "windowType": w.props.get("_WEBOS_WINDOW_TYPE").cloned().unwrap_or_else(|| "_WEBOS_WINDOW_TYPE_CARD".into())}))
            .collect();
        json!({"returnValue": true, "windows": list})
    }

    fn video_windows(&self) -> Vec<Value> {
        self.foreign
            .exported
            .values()
            .filter(|e| !e.contexts.is_empty())
            .map(|e| {
                let app = self.scene.window_of(e.surface).and_then(|w| self.scene.window(w)).map(|w| w.app_id().to_string()).unwrap_or_default();
                let d = e.destination;
                let full = d.w >= crate::OUTPUT_WIDTH as i32 && d.h >= crate::OUTPUT_HEIGHT as i32;
                json!({
                    "windowId": e.window_id, "appId": app, "context": e.contexts.last(),
                    "appWindow": {"x": 0, "y": 0, "width": crate::OUTPUT_WIDTH, "height": crate::OUTPUT_HEIGHT},
                    "fullScreen": full, "isPig": false,
                    "displayOutput": {"x": d.x, "y": d.y, "width": d.w, "height": d.h},
                })
            })
            .collect()
    }

    fn home_ui_state(&self) -> Value {
        let visible = |t: &str| -> Value {
            match t {
                "toast" => json!(0),
                "pincodePrompt" => json!(self.scene.windows().any(|w| w.kind == WindowType::PinCodePrompt && w.map_serial > 0)),
                "alert" => json!(self.scene.windows().any(|w| matches!(w.kind, WindowType::Alert | WindowType::AdvancedAlert) && w.map_serial > 0)),
                _ => json!(false),
            }
        };
        json!({"returnValue": true, "homeUIState": HOME_UI_TYPES.iter().map(|t| json!({"uiType": t, "visibility": visible(t)})).collect::<Vec<_>>()})
    }

    fn recents(&self) -> Vec<Value> {
        self.scene.windows().filter(|w| w.kind.is_card() && !w.in_group && w.map_serial > 0).map(|w| json!({"appId": w.app_id(), "title": w.title})).collect()
    }

    /// `closeByAppId`: every window of that app gets `wl_webos_shell_surface.close`; returns how many.
    pub fn close_app(&mut self, app_id: &str) -> usize {
        let ids: Vec<_> = self.scene.windows().filter(|w| w.app_id() == app_id).map(|w| w.id).collect();
        let mut n = 0;
        for id in ids {
            if let Some(res) = self.webos_shell_resource(id) {
                res.close();
                n += 1;
            }
        }
        self.journal(format!("luna: closeByAppId {app_id} -> close sent to {n} window(s)"));
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_rounds_update_the_interactive_refresh_vote() {
        let display = wayland_server::Display::<State>::new().unwrap();
        let mut state = State::new(display.handle(), crate::config::Config::default(),
            Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        state.shell_rust.engine_connected = true;
        state.shell_rust_handle(crate::shell::Msg::Ui {
            can: None, wants: None, live: None, state: Some(json!({"interactiveVisible": true})),
        });
        assert_eq!(state.config.refresh_hz, 120);
        state.luna_state.power_tokens.extend([(700, "PreparePowerOff"), (701, "PreparePowerOn")]);
        state.luna_event(Event { token: 700, payload: json!({"timestamp": "1", "next sub state": "active standby"}) });
        assert_eq!(state.config.refresh_hz, 60);
        state.luna_event(Event { token: 701, payload: json!({"timestamp": "2", "next sub state": "active"}) });
        assert_eq!(state.config.refresh_hz, 120);
    }

    #[test]
    fn cover_open_routes_real_inbound_and_luna_responses() {
        let display = wayland_server::Display::<State>::new().unwrap();
        let mut state = State::new(display.handle(), crate::config::Config::default(),
            Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        let open = |state: &mut State, payload| state.luna_inbound(Inbound { token: 900, service: "com.webos.coverUI".into(),
            method: "open".into(), payload, subscribe: false, sender: "bootd".into(), cancelled: false });
        let calls = |state: &State| state.luna.history().into_iter().filter_map(|m| match m {
            ToBridge::Outbound(o) => Some(o), _ => None
        }).collect::<Vec<_>>();
        open(&mut state, json!({"firstApp":{},"secondApp":{"id":"com.webos.app.livetv"},"fallback":{"id":"home"}}));
        let first = calls(&state).pop().unwrap();
        assert_eq!(first.payload, json!({"id":"com.webos.app.livetv","params":{"boot":true}}));
        state.luna_event(Event { token: first.token, payload: json!({"returnValue":false}) });
        let fallback = calls(&state).pop().unwrap();
        assert_eq!(fallback.payload, json!({"id":"home","params":{}}));
        state.luna_event(Event { token: fallback.token, payload: json!({"returnValue":false}) });
        assert_eq!(state.luna_method("com.webos.coverUI", "getStatus", &json!({}), "").unwrap()["status"], "failed");
        assert_eq!(calls(&state).len(), 2);
        open(&mut state, json!({"firstApp":{"id":"app","params":{"x":1}},"secondApp":{"id":"other"},"fallback":{"id":"home"}}));
        let good = calls(&state).pop().unwrap();
        assert_eq!(good.payload, json!({"id":"app","params":{"x":1}}));
        state.luna_event(Event { token: good.token, payload: json!({"returnValue":true}) });
        let all = calls(&state);
        assert_eq!(all.len(), 4);
        assert_eq!(all[3].uri, "luna://com.webos.bootManager/generateSignal");
        assert_eq!(all[3].payload, json!({"name":"firstapp-launched"}));
        assert_eq!(state.luna_method("com.webos.coverUI", "getStatus", &json!({}), "").unwrap()["status"], "closed");
        open(&mut state, json!({"firstApp":{"id":"first"},"secondApp":{"id":"second"},"fallback":{"id":"home"}}));
        let first = calls(&state).pop().unwrap();
        state.luna_event(Event { token: first.token, payload: json!({"returnValue":false}) });
        let second = calls(&state).pop().unwrap();
        assert_eq!(second.payload["id"], "second");
        state.luna_method("com.webos.coverUI", "close", &json!({}), "");
        let count = calls(&state).len();
        state.luna_event(Event { token: second.token, payload: json!({"returnValue":false}) });
        assert_eq!(calls(&state).len(), count, "close cancels pending fallback");
        open(&mut state, json!({"inputParams":{"boot":true,"channelId":"7"},"secondApp":{"id":"com.webos.app.hdmi1"}}));
        assert_eq!(calls(&state).pop().unwrap().payload["params"], json!({"boot":true,"channelId":"7"}));
        let count = calls(&state).len();
        let wake = json!({"firstApp":{}, "secondApp":{}, "fallback":{"id":"missing.app"},
            "finalFallback":{"id":"com.webos.app.home"}});
        open(&mut state, wake.clone());
        let saved = calls(&state).pop().unwrap();
        assert_eq!(saved.uri, "luna://com.webos.applicationManager/launch");
        assert_eq!(saved.payload, json!({"id":"missing.app","params":{}}));
        assert_eq!(calls(&state).len(), count + 1);
        state.luna_event(Event { token: saved.token, payload: json!({"returnValue":false,"errorText":"app not found"}) });
        let home = calls(&state).pop().unwrap();
        assert_eq!(home.uri, "luna://com.webos.applicationManager/launch");
        assert_eq!(home.payload, json!({"id":"com.webos.app.home","params":{}}));
        assert_eq!(calls(&state).len(), count + 2);
        state.luna_event(Event { token: home.token, payload: json!({"returnValue":true}) });
        let signal = calls(&state).pop().unwrap();
        assert_eq!(signal.uri, "luna://com.webos.bootManager/generateSignal");
        assert_eq!(signal.payload, json!({"name":"firstapp-launched"}));
        assert_eq!(calls(&state).len(), count + 3);
        assert_eq!(state.luna_method("com.webos.coverUI", "getStatus", &json!({}), "").unwrap()["status"], "closed");

        open(&mut state, wake);
        let saved = calls(&state).pop().unwrap();
        state.luna_event(Event { token: saved.token, payload: json!({"returnValue":false}) });
        let home = calls(&state).pop().unwrap();
        let count = calls(&state).len();
        state.luna_event(Event { token: home.token, payload: json!({"returnValue":false}) });
        assert_eq!(calls(&state).len(), count, "terminal failure must not signal firstapp-launched");
        assert_eq!(state.luna_method("com.webos.coverUI", "getStatus", &json!({}), "").unwrap()["status"], "failed");
    }

    #[test]
    fn power_rounds_are_acked_with_their_timestamp() {
        let mut p = Power::default();
        let (uri, payload) = p.on_push("PreparePowerOff", &json!({"timestamp": "544527936", "next sub state": "active standby"})).unwrap();
        assert!(uri.ends_with("/power2/responsePreparePowerOff"));
        assert_eq!(payload["timestamp"], "544527936");
        assert_eq!(payload["ack"], true);
        assert_eq!(payload["clientName"], "com.webos.surfacemanager");
        assert!(!p.rendering);
        let (uri, _) = p.on_push("PreparePowerOn", &json!({"timestamp": "544600000", "next sub state": "active"})).unwrap();
        assert!(uri.ends_with("/power2/responsePreparePowerOn"));
        assert!(p.rendering);
        assert!(p.on_push("PreparePowerOff", &json!({"state": "ACTIVE"})).is_none(), "a state echo without a timestamp is not a round");
    }

    #[test]
    fn the_answer_to_a_video_call_is_a_journal_line_once_and_the_list_is_capped() {
        let mut s = LunaState::default();
        s.note_video_call(7, "setDisplayWindow".into(), "_ctx_1".into());
        assert!(s.video_answer(8, &json!({"returnValue": true})).is_none(), "another call's answer is not this one's");
        let line = s.video_answer(7, &json!({"returnValue": false, "errorText": "no such context"})).unwrap();
        assert_eq!(line, r#"videooutput: setDisplayWindow for "_ctx_1" answered {"errorText":"no such context","returnValue":false}"#);
        assert!(!line.contains("luna: "), "the gate counts placements by the journal's `luna: setDisplayWindow` lines");
        assert!(s.video_answer(7, &json!({})).is_none(), "once");
        for t in 0..100 {
            s.note_video_call(t, "setDisplayWindowProperty".into(), String::new());
        }
        assert_eq!(s.video_calls.len(), 32, "an answer that never comes does not grow the list");
    }
}
