//! What the remote's keys ask of the system layer (`keys::SystemRequest`), carried out by LumaTV's shell where it draws the
//! surface (`shell.rs`) and with LG's own services otherwise.
//!
//! LG's key chain opens Quick Settings, the input picker, Quick Access and the rest as system windows of its own compositor
//! (`systemUIManager/launch`). `State::system_request` is the one place where a key asks for a surface. The shell client says
//! what it can show now (`can`: the quick panel, the input picker, the notification drawer) and is asked first; the table below
//! is the route for what it declines or does not draw: each goes to the app LG's own filters reach for the same key (the Home Hub
//! for Input's, the accessibility overlay for Mute), or, for the
//! windows that exist only inside LG's compositor, the nearest app that shows the same thing, said in the journal line.

use serde_json::{Value, json};

use crate::State;
use crate::keys::{HoldAction, SystemRequest};

const SETTINGS: &str = "com.palm.app.settings";
const HOME_HUB: &str = "com.webos.app.homeconnect";
const QUICK_ACCESS: &str = "com.webos.app.magicnum";
const ACCESSIBILITY: &str = "com.webos.app.accessibility-overlay";
/// LG's remote support (`LG Remote Service`: LG staff see and control the TV once a code is read out). It opens only while the owner has
/// allowed it in Settings > Privacy, which is the helper's owner-only, confirmed switch: its file below exists. No other route turns it on.
const REMOTE_SERVICE: &str = "com.webos.app.remoteservice";
const REMOTE_SERVICE_ALLOWED: &str = "privacy/remoteservice.allowed";

impl State {
    pub fn system_request(&mut self, request: SystemRequest) {
        match request {
            SystemRequest::QuickSettings => {
                if self.shell_show("quick", false) {
                    self.journal("input: gear tap: Quick Settings -> LumaTV's quick panel".into());
                } else {
                    self.interim("gear tap", "Quick Settings", SETTINGS, None);
                }
            }
            SystemRequest::InputPicker => {
                if self.shell_show("picker", false) {
                    self.journal("input: Input tap: the input picker -> LumaTV's input picker".into());
                } else {
                    self.interim("Input tap", "the input picker", HOME_HUB, None);
                }
            }
            SystemRequest::InputHub => self.launch("Input held", "the Home Hub", HOME_HUB, None),
            SystemRequest::GearHold => self.gear_hold(),
            SystemRequest::Accessibility => self.launch("Mute held", "the accessibility panel", ACCESSIBILITY, Some(json!({"quickMode": true}))),
            SystemRequest::QuickAccess(0) => self.launch("0 held", "the Quick Access editor", QUICK_ACCESS, None),
            SystemRequest::QuickAccess(n) => self.interim(&format!("{n} held"), &format!("Quick Access slot {n}"), QUICK_ACCESS, None),
            SystemRequest::LaunchApp(app) => self.launch("app button", &app.clone(), &app, None),
            SystemRequest::OpenSettings => self.button_shows("settings", "LumaTV's Settings"),
            SystemRequest::OpenNotifications => self.button_shows("drawer", "the notification drawer"),
            SystemRequest::PictureOff => self.picture_off(),
            SystemRequest::Exit => self.back_pressed(),
            SystemRequest::CloseKeyboard => self.close_keyboard("a key asked for the keyboard to close"),
        }
    }

    /// A button the owner set to open one of LumaTV's own surfaces (Settings > Remote & pointer > Buttons). Never LG's: a surface the shell
    /// cannot show now is a press that does nothing, said in the journal.
    fn button_shows(&mut self, kind: &str, shown: &str) {
        if self.shell_show(kind, false) {
            self.journal(format!("input: button: {shown}"));
        } else {
            self.journal(format!("input: button: {shown} cannot be shown now, nothing opened"));
        }
    }

    /// The picture off and the sound on, as the quick panel's Screen off tile does it (LG's `option.screenOff`).
    fn picture_off(&mut self) {
        self.journal("input: button: the picture off".into());
        self.luna_call("luna://com.webos.service.settings/setSystemSettings".into(), json!({"category": "option", "settings": {"screenOff": "on"}}));
    }

    /// The gear held: what the owner chose (theme `remote.gearHold`, Settings > Remote & pointer > Hold gear). LG's own answers to this key are
    /// never used: not its Settings app, and not the hidden AV-reset menu (`tvhotkey`), whose items are in LumaTV's Settings. A choice the shell
    /// cannot show now is a held key that does nothing, said in the journal.
    fn gear_hold(&mut self) {
        let choice = self.keys.gear_hold;
        let (kind, shown) = match choice {
            HoldAction::Settings => ("settings", "LumaTV's Settings"),
            HoldAction::Notifications => ("drawer", "the notification drawer -> LumaTV's shell"),
            HoldAction::Nothing => return self.journal("input: gear held: nothing (the owner's choice)".into()),
        };
        if self.shell_show(kind, false) {
            self.journal(format!("input: gear held: {shown}"));
        } else {
            self.journal(format!("input: gear held: LumaTV's {} cannot be shown now, nothing opened", choice.id()));
        }
    }

    /// Whether a luna call would open LG's Remote Service while the owner has not allowed it: such a call is dropped, whoever made it.
    pub fn remote_service_refused(&self, uri: &str, payload: &Value) -> bool {
        let launch = uri.rsplit_once('/').is_some_and(|(service, method)| {
            service.rsplit('/').next().is_some_and(|s| s.eq_ignore_ascii_case("com.webos.applicationManager") || s.eq_ignore_ascii_case("com.webos.service.applicationmanager"))
                && matches!(method, "launch" | "open" | "launchDefaultApp")
        });
        let names = ["id", "appId"].into_iter().any(|k| payload.get(k).and_then(Value::as_str) == Some(REMOTE_SERVICE));
        launch && names && !self.config.data_dir.join(REMOTE_SERVICE_ALLOWED).is_file()
    }

    /// A launch through the app manager, with the journal line the gate reads.
    fn launch(&mut self, key: &str, what: &str, app: &str, params: Option<Value>) {
        self.journal(format!("input: {key}: {what}, launching {app}"));
        self.launch_app(app, params);
    }

    /// A window that lives only inside LG's compositor and that the shell cannot show now, answered with the app nearest to it.
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
