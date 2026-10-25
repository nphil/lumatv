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
    fn luna_token(&mut self) -> u64 {
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
    }

    /// An answer to one of the compositor's own calls: tvpower pushes and the audio service's volume state are the
    /// ones acted on.
    pub fn luna_event(&mut self, ev: Event) {
        if Some(ev.token) == self.luna_state.audio_token {
            // X4: {"volumeStatus": {"muteStatus": false, "soundOutput": "external_arc", "volume": 0, ...}}
            if let Some(m) = ev.payload.pointer("/volumeStatus/muteStatus").and_then(Value::as_bool) {
                self.luna_state.muted = m;
            }
            return;
        }
        let Some(round) = self.luna_state.power_tokens.iter().find(|(t, _)| *t == ev.token).map(|(_, r)| *r) else {
            log::debug!("luna event token {}: {}", ev.token, ev.payload);
            return;
        };
        if round == "PowerState" {
            self.luna_state.power.on_state(&ev.payload);
            return;
        }
        if let Some((uri, payload)) = self.luna_state.power.on_push(round, &ev.payload) {
            self.journal(format!("tvpower: {round} ({}) -> ack", ev.payload.get("next sub state").and_then(Value::as_str).unwrap_or("")));
            let token = self.luna_token();
            self.luna.send(ToBridge::Outbound(Outbound { token, uri, payload, subscribe: false }));
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

    /// Pushes every subscription whose payload changed. Called after the scene or input state changed.
    pub fn luna_notify(&mut self) {
        if self.luna_state.subscriptions.is_empty() {
            return;
        }
        let mut pushes = Vec::new();
        let subs: Vec<(usize, String, String, String)> =
            self.luna_state.subscriptions.iter().enumerate().map(|(i, s)| (i, s.service.clone(), s.method.clone(), s.sender.clone())).collect();
        for (i, service, method, sender) in subs {
            if let Some(p) = self.luna_method(&service, &method, &json!({}), &sender) {
                let sub = &mut self.luna_state.subscriptions[i];
                if sub.last != p {
                    sub.last = p.clone();
                    if method == "getForegroundAppInfo" {
                        let apps: Vec<&str> = p["foregroundAppInfo"].as_array().into_iter().flatten().filter_map(|a| a["appId"].as_str()).collect();
                        pushes.push((Reply { token: sub.token, payload: p.clone() }, Some(format!("luna: foreground {apps:?} pushed to {sender}"))));
                    } else {
                        pushes.push((Reply { token: sub.token, payload: p }, None));
                    }
                }
            }
        }
        for (r, line) in pushes {
            self.luna.send(ToBridge::Reply(r));
            if let Some(l) = line {
                self.journal(l);
            }
        }
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
            ("com.webos.surfacemanager", "showInputPicker")
            | ("com.webos.surfacemanager", "showQuickSettings")
            | ("com.webos.service.systemUIManager", "launch") => {
                self.journal(format!("luna: {method} {payload} (system UI not built yet)"));
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
    /// mapped app window above it (Settings came as an overlay). Items without an appId are left out; CARD and
    /// RESTRICTED windows that are not members of another window's group carry `multiview`, `pip`, `primary`,
    /// `viewType` and `order`; surface-group owners and members say so (`windowGroupOwner`, `windowGroupOwnerId`).
    /// SAM turns these pushes into its own `foreground`/`background` events. The system UI manager's variant lists
    /// the system UI windows.
    fn foreground_app_info(&self, system_ui: bool) -> Value {
        let mapped = |w: &&crate::window::Window| w.map_serial > 0 && self.surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false);
        let mut items: Vec<&crate::window::Window> = Vec::new();
        if system_ui {
            items.extend(self.scene.windows().filter(mapped).filter(|w| w.kind == WindowType::SystemUi));
        } else {
            items.extend(self.scene.foreground(&self.surfaces));
            let mut above: Vec<&crate::window::Window> = self
                .scene
                .windows()
                .filter(mapped)
                .filter(|w| matches!(w.kind, WindowType::Overlay | WindowType::Floating | WindowType::Popup) && w.state != crate::window::ShellState::Minimized)
                .collect();
            above.sort_by_key(|w| w.map_serial);
            items.extend(above);
        }
        let infos: Vec<Value> = items.into_iter().filter(|w| !w.app_id().is_empty()).map(|w| self.foreground_item(w)).collect();
        json!({"returnValue": true, "mode": "normal", "foregroundAppInfo": infos})
    }

    fn foreground_item(&self, w: &crate::window::Window) -> Value {
        let owner = self.scene.groups.iter().any(|g| g.owner == w.surface);
        let owner_of_group = self.scene.groups.iter().find(|g| {
            g.owner != w.surface && (g.layers.values().any(|l| l.member == Some(w.surface)) || g.anonymous.iter().any(|(s, _)| *s == w.surface))
        });
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
            .filter(|w| self.surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
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
        self.scene.windows().filter(|w| w.kind.is_card() && w.map_serial > 0).map(|w| json!({"appId": w.app_id(), "title": w.title})).collect()
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
}
