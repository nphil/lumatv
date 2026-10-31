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
    /// The audio service's volume subscription, and the mute state it last pushed (what Mute toggles from).
    audio_token: Option<u64>,
    pub muted: bool,
    /// The tokens of the video calls (placements, properties, mute) that have not been answered yet: (token, method, context). Capped: an answer
    /// that never comes must not grow it.
    video_calls: Vec<(u64, String, String)>,
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
            audio_token: None,
            video_calls: Vec::new(),
            muted: false,
        }
    }
}

impl Default for Power {
    fn default() -> Self {
        Power { state: "unknown".into(), rendering: true, pending: Vec::new() }
    }
}

/// tvpower's rounds as the compositor sees them (`StarfishPowerService.qml`): a push with `timestamp` and
/// `next sub state` on one of the three registrations, answered with the matching `response*` carrying the
/// same timestamp and `ack: true`.
pub struct Power {
    pub state: String,
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
        self.pending.push((round.to_string(), ts.clone(), next));
        Some(power::response(round, ts, true))
    }

    pub fn on_state(&mut self, payload: &Value) {
        if let Some(s) = payload.get("state").and_then(Value::as_str) {
            self.state = s.to_string();
        }
    }
}

const HOME_UI_TYPES: [&str; 7] = ["toast", "inputAlert", "alert", "pincodePrompt", "quickSetting", "com.webos.app.voiceagent", "com.webos.app.touchremote"];

const AUDIO: &str = "luna://com.webos.service.audio";

impl State {
    pub(crate) fn luna_token(&mut self) -> u64 {
        self.luna_state.next_token += 1;
        self.luna_state.next_token
    }

    /// Registers with tvpower and subscribes to the audio service's volume state (call once the bridge is up).
    pub fn luna_register(&mut self) {
        let calls = power::register_calls();
        let rounds = ["PreparePowerOff", "PreparePowerOn", "PowerOffLogoRequest", "PowerState"];
        for ((uri, payload), round) in calls.into_iter().zip(rounds) {
            let token = self.luna_token();
            self.luna_state.power_tokens.push((token, round));
            self.luna.send(ToBridge::Outbound(Outbound { token, uri, payload, subscribe: true }));
        }
        let token = self.luna_token();
        self.luna_state.audio_token = Some(token);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: format!("{AUDIO}/master/getVolume"), payload: json!({"subscribe": true}), subscribe: true }));
        // the hardware video windows read the video sinks' status while their switch is on (`vwin`)
        self.vwin_subscribe();
    }

    /// An answer to one of the compositor's own calls: tvpower pushes and the audio service's volume state are the
    /// ones acted on; the shell's calls are answered to it.
    pub fn luna_event(&mut self, ev: Event) {
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
            let was = self.luna_state.power.state.clone();
            self.luna_state.power.on_state(&ev.payload);
            if self.luna_state.power.state != was {
                let line = format!("tvpower: the set is {:?} (was {was:?}){}", self.luna_state.power.state, self.since_power_key());
                self.journal(line);
            }
            return;
        }
        if let Some((uri, payload)) = self.luna_state.power.on_push(round, &ev.payload) {
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
            ("com.webos.coverUI", "getStatus") => json!({"returnValue": true, "status": "closed"}),
            ("com.webos.coverUI", "open") | ("com.webos.coverUI", "close") => ok(),
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
                .filter(|w| matches!(w.kind, WindowType::Overlay | WindowType::Floating | WindowType::Popup) && w.state != crate::window::ShellState::Minimized)
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
