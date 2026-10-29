//! LG's alert feeds under LumaTV's compositor (`panels/Notify.qml` of the QML shell, the alert, the input popup and the PIN prompt; the
//! mocks `docs/mocks/06-confirm-dialog`, `18-no-signal`, `19-pin-prompt`): the three feeds of `com.webos.notification`
//! (`getAlertNotification`, `getInputAlertNotification`, `getPincodePromptNotification`, subscribed through the channel) drawn by
//! LumaTV. Nothing is filtered or delayed: an EAS or thermal alert is shown like any other, as soon as the service pushes it.
//!
//! - **Alert**: a queue, oldest first, the first shown; a modal one goes in front of the ones that are not, one of the same source,
//!   title and message replaces the one up; `close` (with its `timestamp`) and `closeAll` come from the service. A button's `action` is
//!   a luna call the service attached (`serviceURI`, `serviceMethod`, `launchParams`); Back, and a timeout, close it unanswered and run
//!   its `onCloseAction`. A `warning` is drawn over the whole screen.
//! - **Input popup** (a device connected to an input): one at a time; a button per input to switch to it and "Stay here"; Back or its
//!   timeout (15 s unless it names one) closes it.
//! - **PIN prompt**: four digits on a 3 by 4 pad (the remote's number keys type too), OK relays them through `closePincodePrompt`
//!   (`closeType` "relay"), Back takes a digit back and, with none typed, cancels (`closeType` "close"); the third wrong try closes it
//!   as LG's view does.
//!
//! The PIN prompt is over the input popup over the alert (LG's order). While one is up the model wants every key.

use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

pub const ALERT_TAG: u64 = 0x5359_0001;
pub const INPUT_TAG: u64 = 0x5359_0002;
pub const PIN_TAG: u64 = 0x5359_0003;
pub const ALERT_URI: &str = "luna://com.webos.notification/getAlertNotification";
pub const INPUT_URI: &str = "luna://com.webos.notification/getInputAlertNotification";
pub const PIN_URI: &str = "luna://com.webos.notification/getPincodePromptNotification";
pub const CLOSE_PIN_URI: &str = "luna://com.webos.notification/closePincodePrompt";
/// The tags of the one-shot calls the model makes (answers are ignored).
const CALL_BASE: u64 = 0x5359_1000;
const CALL_SPAN: u64 = 0x0000_0800;
/// How long the input popup stays when the service names no timeout.
const INPUT_TIMEOUT_MS: u64 = 15_000;
/// The PIN pad: the keys in reading order, then OK (index 12).
pub const PAD: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "clear", "0", "back"];
pub const OK_INDEX: usize = 12;
/// A wrong PIN the third time closes the prompt.
const TRIES: u32 = 3;

/// A luna call the service attached to a button: the URI with its method, and the parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub uri: String,
    pub params: Value,
}

impl Action {
    fn from_json(v: &Value) -> Option<Action> {
        let base = v.get("serviceURI").and_then(Value::as_str).filter(|s| !s.is_empty())?;
        let method = v.get("serviceMethod").and_then(Value::as_str).unwrap_or("");
        let uri = if method.is_empty() || method.starts_with('/') || base.ends_with('/') { format!("{base}{method}") } else { format!("{base}/{method}") };
        Some(Action { uri, params: v.get("launchParams").cloned().filter(Value::is_object).unwrap_or_else(|| json!({})) })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: String,
    pub action: Option<Action>,
    pub danger: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub timestamp: Value,
    pub source_id: String,
    pub title: String,
    pub message: String,
    pub modal: bool,
    /// LG's protected kind (emergency, thermal, battery): drawn over the whole screen.
    pub warning: bool,
    pub buttons: Vec<Button>,
    pub on_close: Option<Action>,
    /// The focused button.
    pub index: usize,
    pub expires: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputChoice {
    pub app_id: String,
    pub port_name: String,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputPopup {
    pub timestamp: Value,
    pub port_name: String,
    /// A picture of the service's own: a file path, or "" for the kit's cable.
    pub port_icon: String,
    pub inputs: Vec<InputChoice>,
    /// The focused button: one per input, then "Stay here".
    pub index: usize,
    pub expires: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub app_id: String,
    pub title: String,
    pub message: String,
    pub prompt_type: String,
    pub retry: bool,
    pub typed: String,
    /// The focused key: 0..=11 of [`PAD`], 12 OK.
    pub index: usize,
    pub tries: u32,
}

/// What the surface draws: the one on top.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    None,
    Alert(Alert),
    Input(InputPopup),
    Pin(Pin),
}

/// What a press on the surface (a click of the pointer) asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Button `i` of the alert.
    Alert(usize),
    /// Button `i` of the input popup (the last is "Stay here").
    Input(usize),
    /// Key `i` of the PIN pad (12 is OK).
    Pin(usize),
}

#[derive(Debug, Default)]
pub struct Alerts {
    alerts: Vec<Alert>,
    input: Option<InputPopup>,
    pin: Option<Pin>,
    subscribed: bool,
    serial: u64,
    changed: bool,
}

impl Alerts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The three subscriptions, asked once at the start.
    pub fn subscribe(&mut self) -> Vec<Effect> {
        if self.subscribed {
            return Vec::new();
        }
        self.subscribed = true;
        [(ALERT_TAG, ALERT_URI), (INPUT_TAG, INPUT_URI), (PIN_TAG, PIN_URI)]
            .into_iter()
            .map(|(tag, uri)| Effect::Luna { tag, uri: uri.into(), payload: json!({"subscribe": true}), subscribe: true })
            .collect()
    }

    /// An alert, an input popup or a PIN prompt is up: every key is theirs.
    pub fn wants(&self) -> bool {
        !self.alerts.is_empty() || self.input.is_some() || self.pin.is_some()
    }

    pub fn alert_up(&self) -> bool {
        !self.alerts.is_empty()
    }

    pub fn input_up(&self) -> bool {
        self.input.is_some()
    }

    pub fn pin_up(&self) -> bool {
        self.pin.is_some()
    }

    pub fn pin(&self) -> Option<&Pin> {
        self.pin.as_ref()
    }

    pub fn state(&self) -> Value {
        json!({"alerts": self.alerts.len(), "input": self.input.is_some(), "pin": self.pin.is_some()})
    }

    fn call(&mut self, action: &Action, out: &mut Vec<Effect>) {
        self.serial = (self.serial + 1) % CALL_SPAN;
        out.push(Effect::Luna { tag: CALL_BASE + self.serial, uri: action.uri.clone(), payload: action.params.clone(), subscribe: false });
    }

    // ---- the alert

    fn alert(&mut self, m: &Value, now: u64) {
        match m.get("alertAction").and_then(Value::as_str) {
            Some("closeAll") => {
                self.alerts.clear();
                self.changed = true;
            }
            Some("close") => {
                if let Some(ts) = m.get("alertInfo").and_then(|i| i.get("timestamp")) {
                    self.alerts.retain(|a| &a.timestamp != ts);
                    self.changed = true;
                }
            }
            Some("open") => {
                let Some(info) = m.get("alertInfo").filter(|i| i.is_object()) else { return };
                let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let buttons: Vec<Button> = info
                    .get("buttons")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|b| Button {
                        label: b.get("label").and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or("OK").to_string(),
                        action: b.get("action").and_then(Action::from_json),
                        danger: b.get("danger").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect();
                // the last button that says it has the focus keeps it, as the QML's loop leaves it
                let index = info
                    .get("buttons")
                    .and_then(Value::as_array)
                    .and_then(|bs| bs.iter().rposition(|b| b.get("focus").and_then(Value::as_bool).unwrap_or(false)))
                    .unwrap_or(0);
                let timeout = info.get("timeout").and_then(Value::as_f64).filter(|t| *t > 0.0);
                let a = Alert {
                    timestamp: info.get("timestamp").cloned().unwrap_or(Value::Null),
                    source_id: text("sourceId"),
                    title: text("title"),
                    message: text("message"),
                    modal: info.get("modal").and_then(Value::as_bool).unwrap_or(false),
                    warning: info.get("type").and_then(Value::as_str) == Some("warning"),
                    buttons,
                    on_close: info.get("onCloseAction").and_then(Action::from_json),
                    index,
                    expires: timeout.map(|t| now + (t * 1000.0) as u64),
                };
                // one of the same source, title and words is the same alert again
                self.alerts.retain(|d| !(d.source_id == a.source_id && d.title == a.title && d.message == a.message));
                if a.modal {
                    // a modal one goes after the modal ones already waiting, in front of the rest
                    let at = self.alerts.iter().position(|d| !d.modal).unwrap_or(self.alerts.len());
                    self.alerts.insert(at, a);
                } else {
                    self.alerts.push(a);
                }
                self.changed = true;
            }
            _ => {}
        }
    }

    /// Takes the alert at the front away: `unanswered` runs its `onCloseAction` (Back, a timeout), an answer does not.
    fn drop_front(&mut self, unanswered: bool, out: &mut Vec<Effect>) {
        if self.alerts.is_empty() {
            return;
        }
        let a = self.alerts.remove(0);
        if unanswered && let Some(action) = &a.on_close {
            self.call(action, out);
        }
        self.changed = true;
    }

    fn answer_alert(&mut self, i: usize, out: &mut Vec<Effect>) {
        let Some(a) = self.alerts.first() else { return };
        let action = a.buttons.get(i).and_then(|b| b.action.clone());
        if let Some(action) = action {
            self.call(&action, out);
        }
        self.drop_front(false, out);
    }

    // ---- the input popup

    fn input(&mut self, m: &Value, now: u64) {
        match m.get("alertAction").and_then(Value::as_str) {
            Some("close") => {
                let same = match (&self.input, m.get("alertInfo").and_then(|i| i.get("timestamp"))) {
                    (Some(p), Some(ts)) => &p.timestamp == ts,
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if same {
                    self.input = None;
                    self.changed = true;
                }
            }
            Some("open") => {
                let Some(info) = m.get("alertInfo").filter(|i| i.is_object()) else { return };
                let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let launch = |app_id: &str, params: &Value| Action {
                    uri: "luna://com.webos.applicationManager/launch".into(),
                    params: json!({"id": app_id, "params": if params.is_object() { params.clone() } else { json!({}) }}),
                };
                let mut inputs: Vec<InputChoice> = info
                    .get("inputs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|i| {
                        let app_id = text(i, "appId");
                        InputChoice {
                            port_name: text(i, "portName"),
                            action: i.get("action").and_then(Action::from_json).unwrap_or_else(|| launch(&app_id, i.get("params").unwrap_or(&Value::Null))),
                            app_id,
                        }
                    })
                    .collect();
                if inputs.is_empty() && info.get("appId").and_then(Value::as_str).is_some_and(|a| !a.is_empty()) {
                    let app_id = text(info, "appId");
                    inputs.push(InputChoice {
                        port_name: text(info, "portName"),
                        action: info.get("action").and_then(Action::from_json).unwrap_or_else(|| launch(&app_id, info.get("params").unwrap_or(&Value::Null))),
                        app_id,
                    });
                }
                let timeout = info.get("timeout").and_then(Value::as_f64).filter(|t| *t > 0.0).map_or(INPUT_TIMEOUT_MS, |t| (t * 1000.0) as u64);
                self.input = Some(InputPopup {
                    timestamp: info.get("timestamp").cloned().unwrap_or(Value::Null),
                    port_name: text(info, "portName"),
                    port_icon: text(info, "portIcon"),
                    inputs,
                    index: 0,
                    expires: now + timeout,
                });
                self.changed = true;
            }
            _ => {}
        }
    }

    // ---- the PIN prompt

    fn prompt(&mut self, m: &Value, out: &mut Vec<Effect>) {
        match m.get("pincodePromptAction").and_then(Value::as_str) {
            Some("close") => {
                self.pin = None;
                self.changed = true;
            }
            Some("open") => {
                let Some(info) = m.get("pincodePromptInfo").filter(|i| i.is_object()) else { return };
                let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let retry = match info.get("retry") {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
                    _ => false,
                };
                let tries = match &self.pin {
                    Some(old) if old.retry && retry => old.tries + 1,
                    _ if retry => 1,
                    _ => 0,
                };
                self.pin = Some(Pin {
                    app_id: text("appId"),
                    title: text("title"),
                    message: text("message"),
                    prompt_type: text("promptType"),
                    retry,
                    typed: String::new(),
                    index: 0,
                    tries,
                });
                self.changed = true;
                // LG closes the prompt on the third wrong try
                if tries >= TRIES {
                    self.cancel_pin(out);
                }
            }
            _ => {}
        }
    }

    fn relay_pin(&mut self, out: &mut Vec<Effect>) {
        let Some(p) = &self.pin else { return };
        if p.typed.len() < 4 {
            return;
        }
        let mode = if p.prompt_type == "parental" || p.prompt_type.is_empty() { "" } else { p.prompt_type.as_str() };
        let action = Action { uri: CLOSE_PIN_URI.into(), params: json!({"pincode": p.typed, "closeType": "relay", "mode": mode, "appId": p.app_id}) };
        self.call(&action, out);
        if let Some(p) = &mut self.pin {
            p.typed.clear();
        }
        self.changed = true;
    }

    fn cancel_pin(&mut self, out: &mut Vec<Effect>) {
        self.call(&Action { uri: CLOSE_PIN_URI.into(), params: json!({"closeType": "close"}) }, out);
        self.pin = None;
        self.changed = true;
    }

    fn pad_press(&mut self, key: &str) {
        let Some(p) = &mut self.pin else { return };
        match key {
            "clear" => p.typed.clear(),
            "back" => {
                p.typed.pop();
            }
            digit if p.typed.len() < 4 => p.typed.push_str(digit),
            _ => {}
        }
        self.changed = true;
    }

    // ---- keys and presses

    /// Presses what a click of the pointer chose.
    pub fn press(&mut self, target: Target) -> Vec<Effect> {
        let mut out = Vec::new();
        self.changed = false;
        match target {
            Target::Pin(i) if self.pin.is_some() => {
                if let Some(p) = &mut self.pin {
                    p.index = i.min(OK_INDEX);
                }
                if i >= OK_INDEX {
                    self.relay_pin(&mut out)
                } else {
                    self.pad_press(PAD[i])
                }
                self.changed = true;
            }
            Target::Input(i) if self.input.is_some() && self.pin.is_none() => {
                if let Some(p) = &mut self.input {
                    p.index = i.min(p.inputs.len());
                }
                self.act_input(&mut out);
            }
            Target::Alert(i) if self.alert_up() && self.input.is_none() && self.pin.is_none() => {
                if let Some(a) = self.alerts.first_mut() {
                    a.index = i.min(a.buttons.len().saturating_sub(1));
                }
                self.answer_alert(i, &mut out);
            }
            _ => {}
        }
        if self.changed {
            out.push(Effect::Redraw);
        }
        out
    }

    fn act_input(&mut self, out: &mut Vec<Effect>) {
        let Some(p) = self.input.take() else { return };
        if let Some(choice) = p.inputs.get(p.index) {
            self.call(&choice.action.clone(), out);
        }
        self.changed = true;
    }

    fn key(&mut self, name: &str, repeat: bool, out: &mut Vec<Effect>) {
        if let Some(p) = &self.pin {
            let mut i = p.index;
            match name {
                "back" => {
                    if repeat {
                        return;
                    }
                    if p.typed.is_empty() {
                        self.cancel_pin(out)
                    } else {
                        self.pad_press("back")
                    }
                    return;
                }
                "ok" => {
                    if repeat {
                        return;
                    }
                    if i >= OK_INDEX {
                        self.relay_pin(out)
                    } else {
                        self.pad_press(PAD[i])
                    }
                    return;
                }
                "left" => i = if i % 3 == 0 && i < OK_INDEX { i } else { i - 1 },
                "right" => {
                    i = if i % 3 == 2 && i < OK_INDEX {
                        i
                    } else if i < OK_INDEX {
                        i + 1
                    } else {
                        i
                    }
                }
                "up" => {
                    i = if i >= OK_INDEX {
                        10
                    } else if i >= 3 {
                        i - 3
                    } else {
                        i
                    }
                }
                "down" => i = if i < 9 { i + 3 } else { OK_INDEX },
                d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
                    if !repeat {
                        self.pad_press(d);
                    }
                    return;
                }
                _ => return,
            }
            if let Some(p) = &mut self.pin
                && p.index != i
            {
                p.index = i.min(OK_INDEX);
                self.changed = true;
            }
            return;
        }
        if let Some(p) = &mut self.input {
            let n = p.inputs.len() + 1;
            match name {
                "left" => p.index = p.index.saturating_sub(1),
                "right" => p.index = (p.index + 1).min(n - 1),
                "ok" if !repeat => {
                    self.act_input(out);
                }
                "back" if !repeat => {
                    self.input = None;
                }
                _ => return,
            }
            self.changed = true;
            return;
        }
        if let Some(a) = self.alerts.first_mut() {
            let count = a.buttons.len();
            match name {
                "left" => a.index = a.index.saturating_sub(1),
                "right" => a.index = (a.index + 1).min(count.saturating_sub(1)),
                "ok" if !repeat => {
                    let i = a.index;
                    self.answer_alert(i, out);
                }
                "back" if !repeat => self.drop_front(true, out),
                _ => return,
            }
            self.changed = true;
        }
    }
}

impl Model for Alerts {
    type View = View;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        self.changed = false;
        match input {
            Input::Luna { tag, payload } => match *tag {
                ALERT_TAG => self.alert(payload, now),
                INPUT_TAG => self.input(payload, now),
                PIN_TAG => self.prompt(payload, &mut out),
                _ => {}
            },
            Input::Key { name, down: true, repeat } => self.key(name, *repeat, &mut out),
            Input::Tick => {
                // a timeout closes an alert unanswered, an input popup quietly
                while let Some(i) = self.alerts.iter().position(|a| a.expires.is_some_and(|e| e <= now)) {
                    let a = self.alerts.remove(i);
                    if let Some(action) = &a.on_close {
                        self.call(action, &mut out);
                    }
                    self.changed = true;
                }
                if self.input.as_ref().is_some_and(|p| p.expires <= now) {
                    self.input = None;
                    self.changed = true;
                }
            }
            _ => {}
        }
        if self.changed {
            out.push(Effect::Redraw);
        }
        out
    }

    fn view(&self) -> View {
        if let Some(p) = &self.pin {
            View::Pin(p.clone())
        } else if let Some(p) = &self.input {
            View::Input(p.clone())
        } else if let Some(a) = self.alerts.first() {
            View::Alert(a.clone())
        } else {
            View::None
        }
    }

    fn due_at(&self) -> Option<u64> {
        self.alerts.iter().filter_map(|a| a.expires).chain(self.input.iter().map(|p| p.expires)).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAUNCH: &str = r#"{"serviceURI": "luna://com.webos.applicationManager", "serviceMethod": "/launch", "launchParams": {"id": "com.webos.app.hdmi3"}}"#;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn alert_doc(ts: u64, buttons: &str, extra: &str) -> Value {
        parse(&format!(
            r#"{{"alertAction": "open", "alertInfo": {{"sourceId": "com.webos.app.home", "title": "Uninstall Crunchyroll?", "message": "The app is removed.", "modal": true, "timeout": 0, "timestamp": {ts},
            "buttons": {buttons}, "onCloseAction": {{"serviceURI": "luna://com.webos.notification", "serviceMethod": "/closeAlert", "launchParams": {{"timestamp": {ts}}}}} {extra}}}}}"#
        ))
    }

    fn feed(a: &mut Alerts, tag: u64, doc: Value, now: u64) -> Vec<Effect> {
        a.update(&Input::Luna { tag, payload: doc }, now)
    }

    fn key(a: &mut Alerts, name: &str) -> Vec<Effect> {
        a.update(&Input::Key { name: name.into(), down: true, repeat: false }, 0)
    }

    fn calls(e: &[Effect]) -> Vec<(String, Value)> {
        e.iter()
            .filter_map(|e| match e {
                Effect::Luna { uri, payload, subscribe: false, .. } => Some((uri.clone(), payload.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_three_feeds_are_subscribed_once() {
        let mut a = Alerts::new();
        let e = a.subscribe();
        assert_eq!(e.len(), 3);
        assert!(e.iter().all(|e| matches!(e, Effect::Luna { subscribe: true, .. })));
        assert!(a.subscribe().is_empty());
    }

    #[test]
    fn an_alert_takes_the_keys_its_focused_button_starts_focused_and_ok_runs_its_action() {
        let mut a = Alerts::new();
        let doc = alert_doc(1, &format!(r#"[{{"label": "Keep it", "focus": true}}, {{"label": "Uninstall", "action": {LAUNCH}, "danger": true}}]"#), "");
        assert!(feed(&mut a, ALERT_TAG, doc, 1000).contains(&Effect::Redraw));
        assert!(a.wants() && a.alert_up());
        let View::Alert(v) = a.view() else { panic!("an alert is up") };
        assert_eq!(v.index, 0, "the button with focus: true starts focused");
        assert!(v.modal && !v.warning && v.buttons[1].danger);
        key(&mut a, "right");
        let e = key(&mut a, "ok");
        assert_eq!(
            calls(&e),
            vec![("luna://com.webos.applicationManager/launch".to_string(), json!({"id": "com.webos.app.hdmi3"}))],
            "OK on Uninstall ran its action"
        );
        assert!(!a.alert_up() && !a.wants());
    }

    #[test]
    fn back_closes_an_alert_unanswered_and_runs_its_close_action_a_warning_without_buttons_included() {
        let mut a = Alerts::new();
        let doc = alert_doc(2, "[]", r#", "type": "warning""#);
        feed(&mut a, ALERT_TAG, doc, 0);
        let View::Alert(v) = a.view() else { panic!() };
        assert!(v.warning && v.buttons.is_empty(), "drawn and never filtered");
        let e = key(&mut a, "back");
        assert_eq!(calls(&e), vec![("luna://com.webos.notification/closeAlert".to_string(), json!({"timestamp": 2}))]);
        assert!(!a.alert_up());
    }

    #[test]
    fn a_modal_alert_goes_in_front_of_the_rest_the_same_words_replace_and_close_and_close_all_come_from_the_service() {
        let mut a = Alerts::new();
        let plain = |ts: u64, title: &str| {
            parse(&format!(
                r#"{{"alertAction": "open", "alertInfo": {{"sourceId": "s", "title": "{title}", "message": "m", "modal": false, "timestamp": {ts}, "buttons": []}}}}"#
            ))
        };
        feed(&mut a, ALERT_TAG, plain(1, "first"), 0);
        feed(&mut a, ALERT_TAG, plain(2, "second"), 0);
        let modal = |ts: u64, title: &str| {
            parse(&format!(
                r#"{{"alertAction": "open", "alertInfo": {{"sourceId": "s", "title": "{title}", "message": "m", "modal": true, "timestamp": {ts}, "buttons": []}}}}"#
            ))
        };
        feed(&mut a, ALERT_TAG, modal(3, "modal one"), 0);
        feed(&mut a, ALERT_TAG, modal(4, "modal two"), 0);
        let titles = |a: &Alerts| a.alerts.iter().map(|x| x.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&a), vec!["modal one", "modal two", "first", "second"], "the modal ones first, in the order they came");
        feed(&mut a, ALERT_TAG, plain(5, "first"), 0);
        assert_eq!(titles(&a), vec!["modal one", "modal two", "second", "first"], "the same source, title and words is replaced, not stacked");
        feed(&mut a, ALERT_TAG, parse(r#"{"alertAction": "close", "alertInfo": {"timestamp": 3}}"#), 0);
        assert_eq!(titles(&a), vec!["modal two", "second", "first"]);
        feed(&mut a, ALERT_TAG, parse(r#"{"alertAction": "closeAll"}"#), 0);
        assert!(!a.wants());
    }

    #[test]
    fn an_alert_with_a_timeout_closes_unanswered_at_its_time() {
        let mut a = Alerts::new();
        let doc = alert_doc(7, r#"[{"label": "OK", "focus": true}]"#, "").clone();
        let mut doc = doc;
        doc["alertInfo"]["timeout"] = json!(5);
        feed(&mut a, ALERT_TAG, doc, 10_000);
        assert_eq!(a.due_at(), Some(15_000));
        assert!(a.update(&Input::Tick, 14_999).is_empty());
        let e = a.update(&Input::Tick, 15_000);
        assert!(e.contains(&Effect::Redraw));
        assert_eq!(calls(&e).len(), 1, "its close action ran");
        assert!(!a.alert_up());
    }

    fn popup_doc() -> Value {
        parse(&format!(
            r#"{{"alertAction": "open", "alertInfo": {{"sourceId": "com.webos.app.hdmi3", "timeout": 15, "timestamp": 2, "portName": "HDMI 3", "action": {LAUNCH}, "appId": "com.webos.app.hdmi3"}}}}"#
        ))
    }

    #[test]
    fn the_input_popup_switches_or_stays_and_its_own_timeout_closes_it() {
        let mut a = Alerts::new();
        feed(&mut a, INPUT_TAG, popup_doc(), 1000);
        let View::Input(p) = a.view() else { panic!() };
        assert_eq!((p.inputs.len(), p.index, p.port_name.as_str()), (1, 0, "HDMI 3"));
        key(&mut a, "right");
        let e = key(&mut a, "ok");
        assert!(calls(&e).is_empty(), "Stay here launches nothing");
        assert!(!a.input_up());
        feed(&mut a, INPUT_TAG, popup_doc(), 2000);
        let e = key(&mut a, "ok");
        assert_eq!(calls(&e), vec![("luna://com.webos.applicationManager/launch".to_string(), json!({"id": "com.webos.app.hdmi3"}))]);
        feed(&mut a, INPUT_TAG, popup_doc(), 3000);
        assert_eq!(a.due_at(), Some(18_000));
        a.update(&Input::Tick, 18_000);
        assert!(!a.input_up(), "gone at its own timeout");
        feed(&mut a, INPUT_TAG, popup_doc(), 4000);
        feed(&mut a, INPUT_TAG, parse(r#"{"alertAction": "close", "alertInfo": {"timestamp": 2}}"#), 4100);
        assert!(!a.input_up(), "and by the service's close");
    }

    #[test]
    fn an_input_popup_with_no_action_launches_its_app_and_one_with_inputs_has_a_button_each() {
        let mut a = Alerts::new();
        feed(
            &mut a,
            INPUT_TAG,
            parse(r#"{"alertAction": "open", "alertInfo": {"timestamp": 9, "portName": "HDMI 1", "appId": "com.webos.app.hdmi1", "params": {"x": 1}}}"#),
            0,
        );
        let e = key(&mut a, "ok");
        assert_eq!(calls(&e), vec![("luna://com.webos.applicationManager/launch".to_string(), json!({"id": "com.webos.app.hdmi1", "params": {"x": 1}}))]);
        feed(
            &mut a,
            INPUT_TAG,
            parse(
                r#"{"alertAction": "open", "alertInfo": {"timestamp": 10, "inputs": [{"appId": "a", "portName": "HDMI 1"}, {"appId": "b", "portName": "HDMI 2"}]}}"#,
            ),
            0,
        );
        key(&mut a, "right");
        let e = key(&mut a, "ok");
        assert_eq!(calls(&e).len(), 1);
        assert_eq!(calls(&e)[0].1["id"], "b");
    }

    fn pin_doc(retry: u64, ts: u64) -> Value {
        parse(&format!(
            r#"{{"pincodePromptAction": "open", "pincodePromptInfo": {{"appId": "com.webos.app.hdmi3", "title": "Enter the TV PIN", "message": "HDMI 3 is locked.", "promptType": "parental", "retry": {retry}, "timestamp": {ts}}}}}"#
        ))
    }

    #[test]
    fn the_pin_prompt_types_four_digits_relays_them_and_cancels_on_back() {
        let mut a = Alerts::new();
        feed(&mut a, PIN_TAG, pin_doc(0, 3), 0);
        key(&mut a, "1");
        key(&mut a, "2");
        for _ in 0..4 {
            key(&mut a, "down");
        }
        assert_eq!(a.pin().unwrap().index, OK_INDEX, "Down from the pad lands on OK");
        let e = key(&mut a, "ok");
        assert!(calls(&e).is_empty(), "OK with two digits relays nothing");
        assert_eq!(a.pin().unwrap().typed, "12");
        key(&mut a, "3");
        key(&mut a, "4");
        key(&mut a, "5");
        assert_eq!(a.pin().unwrap().typed, "1234", "a fifth digit is not taken");
        let e = key(&mut a, "ok");
        assert_eq!(calls(&e), vec![(CLOSE_PIN_URI.to_string(), json!({"pincode": "1234", "closeType": "relay", "mode": "", "appId": "com.webos.app.hdmi3"}))]);
        assert_eq!(a.pin().unwrap().typed, "", "what was relayed is cleared");
        let e = key(&mut a, "back");
        assert_eq!(calls(&e), vec![(CLOSE_PIN_URI.to_string(), json!({"closeType": "close"}))], "Back with nothing typed cancels");
        assert!(!a.pin_up());
    }

    #[test]
    fn the_pin_pad_moves_as_a_three_by_four_grid_with_ok_under_it_and_back_takes_a_digit_back() {
        let mut a = Alerts::new();
        feed(&mut a, PIN_TAG, pin_doc(0, 3), 0);
        let at = |a: &Alerts| a.pin().unwrap().index;
        key(&mut a, "left");
        assert_eq!(at(&a), 0, "the left edge holds");
        key(&mut a, "right");
        key(&mut a, "right");
        key(&mut a, "right");
        assert_eq!(at(&a), 2, "the right edge holds");
        key(&mut a, "down");
        assert_eq!(at(&a), 5);
        key(&mut a, "up");
        key(&mut a, "up");
        assert_eq!(at(&a), 2, "the top row holds");
        for _ in 0..3 {
            key(&mut a, "down");
        }
        assert_eq!(at(&a), 11, "the last row of the pad");
        key(&mut a, "down");
        assert_eq!(at(&a), OK_INDEX);
        key(&mut a, "up");
        assert_eq!(at(&a), 10, "Up from OK lands on the 0");
        key(&mut a, "7");
        key(&mut a, "8");
        key(&mut a, "back");
        assert_eq!(a.pin().unwrap().typed, "7", "Back takes a digit back while there is one");
        key(&mut a, "ok");
        assert_eq!(a.pin().unwrap().typed, "70", "OK on the 0 types it");
    }

    #[test]
    fn a_wrong_pin_the_third_time_closes_the_prompt() {
        let mut a = Alerts::new();
        feed(&mut a, PIN_TAG, pin_doc(0, 1), 0);
        assert_eq!(a.pin().unwrap().tries, 0);
        feed(&mut a, PIN_TAG, pin_doc(1, 2), 0);
        assert_eq!((a.pin().unwrap().retry, a.pin().unwrap().tries), (true, 1));
        feed(&mut a, PIN_TAG, pin_doc(1, 3), 0);
        assert_eq!(a.pin().unwrap().tries, 2);
        let e = feed(&mut a, PIN_TAG, pin_doc(1, 4), 0);
        assert_eq!(calls(&e), vec![(CLOSE_PIN_URI.to_string(), json!({"closeType": "close"}))], "LG closes it on the third wrong try");
        assert!(!a.pin_up());
    }

    #[test]
    fn the_pin_prompt_is_over_the_input_popup_over_the_alert_and_a_click_presses_what_it_chose() {
        let mut a = Alerts::new();
        feed(&mut a, ALERT_TAG, alert_doc(1, r#"[{"label": "OK"}]"#, ""), 0);
        feed(&mut a, INPUT_TAG, popup_doc(), 0);
        assert!(matches!(a.view(), View::Input(_)));
        feed(&mut a, PIN_TAG, pin_doc(0, 3), 0);
        assert!(matches!(a.view(), View::Pin(_)));
        let e = a.press(Target::Pin(4));
        assert!(e.contains(&Effect::Redraw));
        assert_eq!(a.pin().unwrap().typed, "5", "a click on the 5 types it and focuses it");
        assert_eq!(a.pin().unwrap().index, 4);
        a.press(Target::Pin(9));
        assert_eq!(a.pin().unwrap().typed, "", "Clear");
        a.press(Target::Alert(0));
        assert!(a.alert_up(), "a press for a surface that is not on top does nothing");
        feed(&mut a, PIN_TAG, parse(r#"{"pincodePromptAction": "close"}"#), 0);
        feed(&mut a, INPUT_TAG, parse(r#"{"alertAction": "close"}"#), 0);
        let e = a.press(Target::Alert(0));
        assert!(!a.alert_up(), "{e:?}");
    }

    #[test]
    fn a_held_ok_or_back_is_not_a_second_press_but_an_arrow_repeats() {
        let mut a = Alerts::new();
        feed(&mut a, ALERT_TAG, alert_doc(1, r#"[{"label": "A"}, {"label": "B"}, {"label": "C"}]"#, ""), 0);
        a.update(&Input::Key { name: "right".into(), down: true, repeat: true }, 0);
        a.update(&Input::Key { name: "right".into(), down: true, repeat: true }, 0);
        let View::Alert(v) = a.view() else { panic!() };
        assert_eq!(v.index, 2);
        assert!(a.update(&Input::Key { name: "ok".into(), down: true, repeat: true }, 0).is_empty());
        assert!(a.alert_up());
        assert!(a.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 0).is_empty(), "a release is nothing");
    }
}
