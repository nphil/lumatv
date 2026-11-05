//! LG's guide is never shown while LumaTV's UI can answer for it.
//!
//! Every way into LG's guide is a launch of `com.webos.app.livemenu` through SAM: Live TV's `inputcommon` makes it for the Guide key (`activateType`
//! `tvguide`), the List, channel-list and Live TV menu keys (`channels`), the banner's Guide button (`tvguide`) and a toast's link (`channels`); the
//! TV Scheduler app's guide button and the user guide's try-now links make it too (read off the set's QML, 2026-11-04). The parameters are told to the
//! app alone: SAM's life events name the app and carry none, and the window has none. They are not needed. Both tabs these launches ask for are the
//! one guide LumaTV has, and the Scheduler is an app of its own (`com.webos.app.scheduler`), which is not touched here; no caller opens the guide's own
//! Scheduler tab.
//!
//! While the UI is on the engine and lists the guide among what it can show (`State::shell_rust_sync_guide`) the scene leaves LG's guide window out of
//! the stack (`Scene::set_guide_held`): from its first commit it is in no frame, has no keyboard focus and is under no pointer. SAM and the apps that
//! watch the foreground list still see it as any overlay, so a launch runs the flow they know. Two things answer the launch:
//!
//! * SAM's `launch` life event for the app, 1 to 3 ms after Live TV asked for it. A cold start maps a window 2.6 s later (2026-11-04) and its
//!   picture-in-picture takes the live picture 0.7 s after that, so this is the moment to act: the UI is asked to show its guide, the door
//!   `ctl shell show guide` uses, and SAM is asked to close the app (`closeByAppId`) before it has a window;
//! * the window, when it maps: SAM is asked to close the app again, and the UI is asked for its guide now if the launch was not heard.
//!
//! With the UI not there (never started, crashed, restarting, or the compositor given up for LG's) nothing is held and nothing is asked: LG's guide
//! opens as stock.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::State;
use crate::luna::{Outbound, ToBridge};
use crate::scene::LG_GUIDE_APP;
use crate::window::WindowId;

/// SAM's app life events: one for each launch, foreground, background, pause and stop of any app, naming it, with no launch parameters.
const LIFE_EVENTS: &str = "luna://com.webos.applicationManager/getAppLifeEvents";
/// How SAM answers the call depends on the caller: the app's own close (Back) ran `PauseAsPolicy`, the app hides and its process stays; the memory
/// manager's close of the same app ran `CloseAsPolicy`, a `close` event and a kill timer (the set's SAM log, 2026-11-04).
const CLOSE: &str = "luna://com.webos.applicationManager/closeByAppId";
/// A window that maps this soon after a launch SAM reported belongs to that launch, which has asked the UI already. A cold start took 2.6 s.
const SAME_LAUNCH: Duration = Duration::from_secs(15);
/// Closes that SAM has not answered, kept to be named in the journal when it does.
const CLOSES_KEPT: usize = 8;

#[derive(Default)]
pub struct Answer {
    /// The token of the life-event subscription.
    life: Option<u64>,
    /// When the UI was last asked for its guide because SAM launched LG's, until the window of that launch maps.
    asked: Option<Instant>,
    /// The tokens of the closes SAM has not answered.
    closing: Vec<u64>,
}

/// Whether `payload` is SAM's `launch` event for LG's guide. (`splash` comes a moment before it, `foreground` and `pause` after: neither is a launch.)
fn launches_guide(payload: &Value) -> bool {
    payload.get("event").and_then(Value::as_str) == Some("launch") && payload.get("appId").and_then(Value::as_str) == Some(LG_GUIDE_APP)
}

impl State {
    /// Subscribes to SAM's life events (once the bridge is up).
    pub(crate) fn lg_guide_subscribe(&mut self) {
        let token = self.luna_token();
        self.luna_state.guide.life = Some(token);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: LIFE_EVENTS.into(), payload: json!({"subscribe": true}), subscribe: true }));
    }

    /// A reply to one of the compositor's own calls that is about LG's guide: an event of the life subscription, or the answer to a close. Returns
    /// whether the token was one of these.
    pub(crate) fn lg_guide_event(&mut self, token: u64, payload: &Value) -> bool {
        if self.luna_state.guide.life == Some(token) {
            if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
                self.luna_state.guide.life = None;
                self.journal(format!("lg-guide: SAM refused the life event subscription ({payload}); a launch is answered when its window maps"));
            } else {
                if launches_guide(payload) {
                    self.lg_guide_launched_at(Instant::now());
                }
                self.home_life_event(payload);
            }
            return true;
        }
        if let Some(i) = self.luna_state.guide.closing.iter().position(|t| *t == token) {
            self.luna_state.guide.closing.swap_remove(i);
            self.journal(format!("lg-guide: SAM answered closeByAppId: {payload}"));
            return true;
        }
        false
    }

    /// SAM launches LG's guide: the UI is asked for its own, now, and the app is closed before it has had the time to map a window.
    pub(crate) fn lg_guide_launched_at(&mut self, now: Instant) {
        if self.scene.guide_held() {
            self.luna_state.guide.asked = Some(now);
            self.lg_guide_show("SAM launched it");
            self.lg_guide_close("SAM launched it");
        }
    }

    /// A window of LG's guide mapped (or already was when the UI came): see [`State::lg_guide_mapped_at`].
    pub(crate) fn lg_guide_mapped(&mut self, wid: WindowId) {
        self.lg_guide_mapped_at(wid, Instant::now());
    }

    /// A window of LG's guide is mapped while the UI answers for it. The scene does not draw it, so there is nothing to hide; SAM is asked to close
    /// the app again, and the UI is asked for its guide unless the launch's event did already.
    pub(crate) fn lg_guide_mapped_at(&mut self, wid: WindowId, now: Instant) {
        if !self.scene.guide_held() {
            return;
        }
        let heard = self.luna_state.guide.asked.take().is_some_and(|t| now.saturating_duration_since(t) < SAME_LAUNCH);
        if !heard {
            self.lg_guide_show("its window mapped");
        }
        self.journal(format!("lg-guide: window {} of LG's guide is mapped and is not drawn", wid.0));
        self.lg_guide_close("its window mapped");
    }

    /// The hold starts: a window of LG's guide that is already up (the UI came, or came back, over it) is answered like one that just mapped.
    pub(crate) fn lg_guide_up(&mut self) {
        let up: Vec<WindowId> = self
            .scene
            .windows()
            .filter(|w| w.app_id() == LG_GUIDE_APP && w.map_serial > 0 && self.surfaces.get(w.surface).is_some_and(|s| s.is_mapped()))
            .map(|w| w.id)
            .collect();
        for id in up {
            self.lg_guide_mapped(id);
        }
    }

    fn lg_guide_show(&mut self, why: &str) {
        if self.shell_show("guide", false) {
            self.journal(format!("lg-guide: LG's guide launched ({why}); LumaTV's guide asked for"));
        }
    }

    /// SAM is asked to close LG's guide; its answer is journaled (`lg_guide_event`).
    fn lg_guide_close(&mut self, why: &str) {
        let token = self.luna_token();
        let closing = &mut self.luna_state.guide.closing;
        if closing.len() >= CLOSES_KEPT {
            closing.remove(0);
        }
        closing.push(token);
        self.journal(format!("lg-guide: closeByAppId asked ({why})"));
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: CLOSE.into(), payload: json!({"id": LG_GUIDE_APP}), subscribe: false }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state whose UI is on the engine and can show its guide.
    fn answering() -> (State, wayland_server::Display<State>) {
        let display = wayland_server::Display::<State>::new().unwrap();
        let mut s = State::new(display.handle(), crate::config::Config::default(), Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust.kinds = vec!["guide".into()];
        s.shell_rust_handle(crate::shell::Msg::Connected);
        s.shell_rust.engine_connected = true;
        s.shell_rust_handle(crate::shell::Msg::Ui { can: Some(vec!["guide".into()]), wants: None, live: None, state: None });
        assert!(s.scene.guide_held());
        (s, display)
    }

    #[test]
    fn a_launch_asks_the_ui_once_and_a_window_that_comes_long_after_it_is_another_launch() {
        let (mut s, _display) = answering();
        let shows = |s: &State| s.shell_rust.events().iter().filter(|e| e["topic"] == "show").count();
        let closes = |s: &State| s.luna.history().iter().filter(|m| matches!(m, ToBridge::Outbound(o) if o.uri == CLOSE)).count();
        let t = Instant::now();
        s.lg_guide_launched_at(t);
        assert_eq!((shows(&s), closes(&s)), (1, 1), "the launch asks the UI for its guide and SAM to close the app, before there is a window");
        s.lg_guide_mapped_at(WindowId(5), t + Duration::from_millis(2_600));
        assert_eq!((shows(&s), closes(&s)), (1, 2), "a cold start maps its window 2.6 s on: the launch asked already, the window closes the app again");
        s.lg_guide_mapped_at(WindowId(5), t + Duration::from_secs(40));
        assert_eq!((shows(&s), closes(&s)), (2, 3), "a window with no launch before it asks for itself");
        s.lg_guide_launched_at(t + Duration::from_secs(100));
        s.lg_guide_mapped_at(WindowId(5), t + Duration::from_secs(130));
        assert_eq!((shows(&s), closes(&s)), (4, 5), "a launch heard and a window 30 s later: that window is no part of the launch and asks for itself");
    }
}
