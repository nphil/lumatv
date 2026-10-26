//! What the remote's keys ask of the system layer (`keys::SystemRequest`), carried out with LG's own services.
//!
//! LG's key chain opens Quick Settings, the input picker, Quick Access and the rest as system windows of its own compositor
//! (`systemUIManager/launch`); LumaTV's compositor draws none of them yet. `State::system_request` is the one place where a key asks
//! for a surface: LumaTV's own UI takes the requests it can draw there once the UI channel exists, and the table below stays as the
//! route for the ones it declines. Until then each goes to the app LG's own filters reach for the same key (Settings for the gear's
//! hold, the Home Hub for Input's, `tvhotkey` for the AV reset, the accessibility overlay for Mute), or, for the windows that exist
//! only inside LG's compositor, the nearest app that shows the same thing, said in the journal line.

use serde_json::{Value, json};

use crate::State;
use crate::keys::SystemRequest;

const SETTINGS: &str = "com.palm.app.settings";
const HOME_HUB: &str = "com.webos.app.homeconnect";
const QUICK_ACCESS: &str = "com.webos.app.magicnum";
const ACCESSIBILITY: &str = "com.webos.app.accessibility-overlay";
const AV_RESET: &str = "com.webos.app.tvhotkey";

impl State {
    pub fn system_request(&mut self, request: SystemRequest) {
        match request {
            SystemRequest::QuickSettings => self.interim("gear tap", "Quick Settings", SETTINGS, None),
            SystemRequest::InputPicker => self.interim("Input tap", "the input picker", HOME_HUB, None),
            SystemRequest::InputHub => self.launch("Input held", "the Home Hub", HOME_HUB, None),
            SystemRequest::Settings => {
                if self.foreground_app() == SETTINGS {
                    self.journal(format!("input: gear held: Settings is in front, closing {SETTINGS}"));
                    self.luna_call("luna://com.webos.applicationManager/closeByAppId".into(), json!({"id": SETTINGS}));
                } else {
                    self.launch("gear held", "Settings", SETTINGS, None);
                }
            }
            SystemRequest::AvReset => self.launch("gear tapped 3 times", "the AV reset menu", AV_RESET, Some(json!({"activateType": "mute-hidden-action"}))),
            SystemRequest::Accessibility => self.launch("Mute held", "the accessibility panel", ACCESSIBILITY, Some(json!({"quickMode": true}))),
            SystemRequest::QuickAccess(0) => self.launch("0 held", "the Quick Access editor", QUICK_ACCESS, None),
            SystemRequest::QuickAccess(n) => self.interim(&format!("{n} held"), &format!("Quick Access slot {n}"), QUICK_ACCESS, None),
            SystemRequest::LaunchApp(app) => self.launch("app button", &app.clone(), &app, None),
            SystemRequest::Exit => self.back_pressed(),
            SystemRequest::CloseKeyboard => self.close_keyboard("a key asked for the keyboard to close"),
        }
    }

    /// A launch through the app manager, with the journal line the gate reads.
    fn launch(&mut self, key: &str, what: &str, app: &str, params: Option<Value>) {
        self.journal(format!("input: {key}: {what}, launching {app}"));
        self.launch_app(app, params);
    }

    /// A window that lives only inside LG's compositor, answered with the app nearest to it until LumaTV's UI draws it.
    fn interim(&mut self, key: &str, what: &str, app: &str, params: Option<Value>) {
        self.journal(format!("input: {key}: {what} has no window of its own here yet, launching {app} in its place"));
        self.launch_app(app, params);
    }

    fn launch_app(&mut self, app: &str, params: Option<Value>) {
        let mut payload = json!({"id": app});
        if let Some(p) = params {
            payload["params"] = p;
        }
        self.luna_call("luna://com.webos.applicationManager/launch".into(), payload);
    }
}
