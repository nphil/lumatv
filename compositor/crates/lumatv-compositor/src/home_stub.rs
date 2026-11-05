//! The native Home stub (`homestub/homestub.c`, `docs/research/boot-direct.md` (g)) from the compositor's side.
//!
//! The stub is a card of LG's Home id with one transparent pixel, which SAM starts as a native app that never registers with it. What SAM does
//! with a launch of such an app is not something to lean on (the set, 2026-11-05): the Home key made it end the process and start another, 0.77 s
//! from the key to the foreground change, and a `launch` of Home over an app did nothing at all, so Home never came back. The compositor raises
//! the stub's card itself, as it brings back any app it has a card for (`front_app`):
//!
//! * the Home key: the card is raised in the key's own pass and SAM is not asked, so SAM never restarts the stub for it;
//! * SAM's `launch` and `relaunch` life events for Home, which is all the compositor sees of a launch by anybody else (`luna-send`, the voice
//!   service, CEC, `last_input_handler`). A `preload` event is no launch;
//! * a card the stub covered stays covered when the stub's card goes (`Window::held_by_home`): SAM ends the stub and starts it again for some
//!   launches, and without this the app under it showed for the moments between.
//!
//! LG's own Home and the web stub are left as they were: the program says what it is with a window property, which nothing else sets.

use serde_json::Value;

use crate::State;
use crate::scene::HOME_APP;
use crate::server::webos_shell::front_app;
use crate::window::{Window, WindowId};

/// The window property `homestub.c` sets, and its value.
pub const STUB_PROPERTY: &str = "lumatv.homeStub";
pub const STUB_VALUE: &str = "native";

/// Whether `w` is the native stub's card.
pub fn is_stub(w: &Window) -> bool {
    w.kind.is_card() && w.app_id() == HOME_APP && w.props.get(STUB_PROPERTY).is_some_and(|v| v == STUB_VALUE)
}

/// Whether `payload` is a SAM life event for a launch of Home: `launch`, and `relaunch` (which carries the launch parameters) unless they are a
/// preload's. A `preload` event, `splash`, `foreground` and `background` are not launches.
pub fn launches_home(payload: &Value) -> bool {
    payload.get("appId").and_then(Value::as_str) == Some(HOME_APP)
        && match payload.get("event").and_then(Value::as_str) {
            Some("launch") => true,
            Some("relaunch") => payload.pointer("/parameters/preload").is_none(),
            _ => false,
        }
}

impl State {
    /// The stub's card when it is there: mapped and one of the cards, in front or covered.
    pub(crate) fn native_home_card(&self) -> Option<WindowId> {
        self.scene
            .windows()
            .filter(|w| is_stub(w) && !w.in_group && w.front > 0 && w.map_serial > 0 && self.surfaces.get(w.surface).is_some_and(|s| s.is_mapped()))
            .max_by_key(|w| w.front)
            .map(|w| w.id)
    }

    /// The Home key while the stub's card is there: it is raised now (nothing to do when it is in front) and SAM is not asked, which would end
    /// the stub and start it again. Returns whether the key was answered here.
    pub(crate) fn native_home_key(&mut self, from_network: bool) -> bool {
        let Some(card) = self.native_home_card() else { return false };
        let key = if from_network { "Home (network key)" } else { "Home" };
        if self.scene.foreground(&self.surfaces).is_some_and(|w| w.id == card) {
            self.journal(format!("input: {key}: Home's card is in front, SAM not asked"));
        } else {
            self.journal(format!("input: {key}: the stub's card raised, SAM not asked"));
            front_app(self, HOME_APP);
        }
        true
    }

    /// One of SAM's life events (`lg_guide.rs` holds the subscription). A launch of Home raises the stub's card; with no stub it is SAM's alone.
    pub(crate) fn home_life_event(&mut self, payload: &Value) {
        if payload.get("appId").and_then(Value::as_str) != Some(HOME_APP) {
            return;
        }
        let event = payload.get("event").and_then(Value::as_str).unwrap_or("");
        if event == "preload" {
            self.journal("home: SAM preloads Home: nothing raised".into());
            return;
        }
        if !launches_home(payload) {
            return;
        }
        let Some(card) = self.native_home_card() else { return };
        if self.scene.foreground(&self.surfaces).is_some_and(|w| w.id == card) {
            self.journal(format!("home: SAM said {event} for Home: its card is in front"));
        } else {
            self.journal(format!("home: SAM said {event} for Home: the stub's card raised"));
            front_app(self, HOME_APP);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::surface::SurfaceId;

    fn window(app: &str, announced: bool) -> Window {
        let mut w = Window::new(WindowId(1), SurfaceId(1));
        w.props.insert("appId".into(), app.into());
        if announced {
            w.props.insert(STUB_PROPERTY.into(), STUB_VALUE.into());
        }
        w
    }

    #[test]
    fn only_a_card_of_homes_id_that_says_it_is_the_stub_is_the_stub() {
        assert!(is_stub(&window(HOME_APP, true)));
        assert!(!is_stub(&window(HOME_APP, false)), "LG's Home, or the web stub");
        assert!(!is_stub(&window("com.netflix.ninja", true)), "the id is Home's");
        let mut overlay = window(HOME_APP, true);
        overlay.kind = crate::window::WindowType::Overlay;
        assert!(!is_stub(&overlay), "a card");
        let mut wrong = window(HOME_APP, false);
        wrong.props.insert(STUB_PROPERTY.into(), "web".into());
        assert!(!is_stub(&wrong), "the value is native");
    }

    #[test]
    fn sams_launch_and_relaunch_of_home_are_launches_and_nothing_else_is() {
        let event = |event: &str, app: &str| json!({"event": event, "appId": app, "returnValue": true, "reason": "undefined"});
        assert!(launches_home(&event("launch", HOME_APP)));
        let home_key = json!({"event": "relaunch", "appId": HOME_APP, "reason": "undefined", "parameters": {"launchType": "homeKey", "activateType": "home"}});
        assert!(launches_home(&home_key), "the Home key's relaunch, as SAM's life events carried it (the set's dump, x4)");
        let preload = json!({"event": "relaunch", "appId": HOME_APP, "parameters": {"preload": "full"}});
        assert!(!launches_home(&preload));
        for other in ["preload", "splash", "foreground", "background", "pause", "stop", "close"] {
            assert!(!launches_home(&event(other, HOME_APP)), "{other}");
        }
        assert!(!launches_home(&event("launch", "com.netflix.ninja")), "another app's");
        assert!(!launches_home(&json!({"returnValue": true, "subscribed": true})), "the subscription's own answer");
    }
}
