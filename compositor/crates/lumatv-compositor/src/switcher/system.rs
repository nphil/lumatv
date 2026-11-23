//! The System menu's three actions: Restart LumaTV, LG Home and Restart TV (`taskmgr/action.rs`). Each is asked by the shell (`system`) or the
//! basic screen after its own confirm, closes the open apps first (a stopped one is ended; the rest have [`QUIT_WAIT`] at most) and is then
//! sent to `lumatv-sm`, the helper that outlives the compositor, over the bridge: it ends and starts LumaTV (`Restart`), hands the screen to
//! LG (`Fallback`) or reboots the set (`Reboot`). The compositor cannot do these itself: the first two end it, and the third is the helper's
//! alone to do.
//!
//! The pixel refresher (the panel's own maintenance, `GET /v1/panel/refresher`) is never interrupted: Restart TV is refused while it runs, on
//! the shell's request before the loop sees it ([`refusal`]), again here, and by the helper once more.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::QUIT_WAIT;
use crate::luna::ToBridge;
use crate::State;
use crate::taskmgr::Action;
use crate::taskmgr::stats::refresher_running;

/// The helper has this long to end LumaTV or the set once it was told; the screen says it did not start after that.
const HELPER_WAIT: Duration = Duration::from_secs(20);
/// What the helper's journal calls the asker.
const REASON: &str = "the task manager";

/// An action under way.
pub(super) struct Going {
    pub(super) action: Action,
    since: Instant,
    /// When the helper was told: until then the apps are being closed and the action can still be called off.
    sent: Option<Instant>,
}

/// The answer to a `system` request that the compositor refuses, which the shell is given in the reply to its POST (the HTTP thread
/// answers it, `shell.rs`): `None` for anything else, and for an action that is allowed. `refresher` is whether the panel's refresher runs;
/// it is read only for the action it matters to.
pub(crate) fn refusal(body: &Value, refresher: impl FnOnce() -> bool) -> Option<Value> {
    if body.get("op").and_then(Value::as_str) != Some("system") {
        return None;
    }
    match body.get("action").and_then(Value::as_str).and_then(Action::parse) {
        None => Some(json!({"ok": false, "error": "action"})),
        Some(Action::RestartTv) if refresher() => Some(json!({"ok": false, "error": "refresher"})),
        Some(_) => None,
    }
}

impl State {
    /// `{"op": "system", "action": "restart-lumatv" | "lg-home" | "reboot"}`.
    pub(super) fn switcher_system(&mut self, body: &Value, now: Instant) {
        let word = body.get("action").and_then(Value::as_str).unwrap_or("");
        match Action::parse(word) {
            Some(action) => {
                if let Err(why) = self.system_action(action, now) {
                    self.journal(format!("switcher: {word}: refused ({why})"));
                }
            }
            None => self.journal(format!("switcher: the shell asked for the system action {word:?}, which is none of them")),
        }
    }

    /// Starts an action: the apps are closed first. `Err` says why not: it is not open, another action is under way, or the refresher runs.
    pub(super) fn system_action(&mut self, action: Action, now: Instant) -> Result<(), &'static str> {
        if !self.switcher.accepting() {
            return Err("closed");
        }
        if self.switcher.system.is_some() {
            return Err("busy");
        }
        if action == Action::RestartTv && refresher_running() {
            return Err("refresher");
        }
        self.journal(format!("switcher: {}: closing the apps first", action.word()));
        self.switcher.system = Some(Going { action, since: now, sent: None });
        self.switcher_quit_all(now);
        Ok(())
    }

    /// Every frame while one is under way: tells the helper once the apps are closed (or have had their time), and gives up on a helper that
    /// did nothing. Whether the topics changed.
    pub(super) fn system_step(&mut self, now: Instant, apps_left: usize) -> bool {
        let Some(g) = &self.switcher.system else { return false };
        let (action, since, sent) = (g.action, g.since, g.sent);
        match sent {
            None if apps_left == 0 || now.saturating_duration_since(since) >= QUIT_WAIT => {
                self.system_send(action, now);
                true
            }
            Some(at) if now.saturating_duration_since(at) >= HELPER_WAIT => {
                self.journal(format!("switcher: {}: the helper did nothing in {} s", action.word(), HELPER_WAIT.as_secs()));
                self.switcher.system = None;
                self.switcher.basic.notice = Some(format!("{} did not start.", action.label()));
                true
            }
            _ => false,
        }
    }

    fn system_send(&mut self, action: Action, now: Instant) {
        let reason = REASON.to_string();
        let line = match action {
            Action::RestartLumaTv => ToBridge::Restart { reason },
            Action::LgHome => ToBridge::Fallback { reason },
            Action::RestartTv => {
                if refresher_running() {
                    self.journal("switcher: reboot: the panel's refresher started meanwhile; not sent".into());
                    self.switcher.system = None;
                    self.switcher.basic.notice = Some("Restart TV waits for the panel's pixel refresher to finish.".into());
                    return;
                }
                ToBridge::Reboot { reason }
            }
        };
        self.journal(format!("switcher: {}: asking the helper", action.word()));
        self.luna.send(line);
        if let Some(g) = &mut self.switcher.system {
            g.sent = Some(now);
        }
    }

    /// The task manager closes: an action the helper was not told of is called off. One it has been told of is the helper's from then on.
    pub(super) fn system_cancel(&mut self) {
        if let Some(g) = self.switcher.system.take()
            && g.sent.is_none()
        {
            self.journal(format!("switcher: {}: called off, the task manager closed", g.action.word()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn only_a_reboot_while_the_refresher_runs_and_an_action_that_is_none_are_refused() {
        let ask = |action: &str, refresher: bool| refusal(&json!({"op": "system", "action": action}), || refresher);
        assert_eq!(ask("reboot", true), Some(json!({"ok": false, "error": "refresher"})));
        assert_eq!(ask("reboot", false), None);
        assert_eq!(ask("restart-lumatv", true), None, "ending LumaTV does not touch the panel's refresher");
        assert_eq!(ask("lg-home", true), None);
        assert_eq!(ask("format", false), Some(json!({"ok": false, "error": "action"})));
        assert_eq!(refusal(&json!({"op": "quit", "id": "x"}), || true), None, "other requests are not its business");
    }

    #[test]
    fn the_refresher_is_read_only_for_the_action_it_matters_to() {
        let reads = Cell::new(0);
        refusal(&json!({"op": "system", "action": "lg-home"}), || {
            reads.set(reads.get() + 1);
            true
        });
        assert_eq!(reads.get(), 0);
    }
}
