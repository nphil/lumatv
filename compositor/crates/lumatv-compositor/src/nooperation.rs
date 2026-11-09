//! LG's NoOp timers are executed by the compositor, not by nopd itself.
//!
//! The screen-saver timer (`[TIMER]ScreenSaver:<seconds>`, 180 s on this set; LG's `option.screenSaverTimer` offers only 3, 10, 20 or
//! 30 min) runs at LumaTV's own wait instead (`screensaver.afterMin`, [`Timers::set_saver_after`]; never with it Off or the screensaver's
//! style Off): nopd still decides what its expiry does, and nothing of LG's is written. Every other timer runs at LG's interval.

use std::time::{Duration, Instant};

use serde_json::Value;

pub(crate) const TIMER_INFO: &str = "luna://com.webos.service.nop/getTimerInfo";
pub(crate) const TIMER_EXPIRED: &str = "luna://com.webos.service.nop/notifyExpireTimer";
/// The kind nopd gives the screen-saver timer, followed by `:` and its interval in seconds.
const SAVER_KIND: &str = "[TIMER]ScreenSaver";

struct Timer {
    kind: String,
    /// LG's interval: with the kind and the flags, what tells a timer from another in nopd's snapshots.
    interval: Duration,
    once: bool,
    repeat: bool,
    /// When its countdown began; `None` for a `once` timer that has fired, until nopd re-adds it.
    since: Option<Instant>,
    seen: bool,
}

pub(crate) struct Timers {
    pub token: Option<u64>,
    timers: Vec<Timer>,
    next: Option<Instant>,
    /// The screen-saver timer's wait; `None` (Off) never lets it expire.
    saver_after: Option<Duration>,
}

impl Default for Timers {
    fn default() -> Self {
        Timers { token: None, timers: Vec::new(), next: None, saver_after: Some(Duration::from_secs(5 * 60)) }
    }
}

impl Timers {
    pub fn deadline(&self) -> Option<Instant> { self.next }

    /// LumaTV's wait for the screen-saver timer. A change applies to the countdown already running.
    pub fn set_saver_after(&mut self, after: Option<Duration>) {
        if self.saver_after != after {
            self.saver_after = after;
            self.reschedule();
        }
    }

    fn wait(&self, t: &Timer) -> Option<Duration> {
        if t.kind.starts_with(SAVER_KIND) { self.saver_after } else { Some(t.interval) }
    }

    fn due(&self, t: &Timer) -> Option<Instant> {
        t.since?.checked_add(self.wait(t)?)
    }

    pub fn update(&mut self, payload: &Value, now: Instant) {
        let Some(timers) = payload.get("timers").and_then(Value::as_array) else { return; };
        for timer in &mut self.timers { timer.seen = false; }
        for value in timers {
            let Some(kind) = value.get("type").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue; };
            let Some(seconds) = value.get("interval").and_then(Value::as_u64).filter(|n| *n > 0) else { continue; };
            let Some(once) = value.get("once").and_then(Value::as_bool) else { continue; };
            let Some(repeat) = value.get("repeat").and_then(Value::as_bool) else { continue; };
            let interval = Duration::from_secs(seconds);
            if now.checked_add(interval).is_none() { continue; }
            // TimerMgr publishes complete lists, including unchanged timers. Its reset is a
            // remove/readd; resetting every snapshot would indefinitely postpone other timers.
            let existing = self.timers.iter().position(|t| t.kind == kind && t.interval == interval
                && t.once == once && t.repeat == repeat);
            if let Some(index) = existing {
                self.timers[index].seen = true;
            } else {
                self.timers.push(Timer { kind: kind.into(), interval, once, repeat, since: Some(now), seen: true });
            }
        }
        self.timers.retain(|timer| timer.seen);
        self.reschedule();
    }

    pub fn activity(&mut self, now: Instant) {
        for timer in &mut self.timers {
            if timer.since.is_some() { timer.since = Some(now); }
        }
        self.reschedule();
    }

    pub fn expired(&mut self, now: Instant) -> Option<&str> {
        if self.next.is_none_or(|due| due > now) { return None; }
        let index = self.timers.iter().position(|t| self.due(t).is_some_and(|due| due <= now))?;
        let timer = &mut self.timers[index];
        // `once` controls LG's local QTimer. `repeat` instead controls nopd's
        // ownership/removal after the notification; it is not the single-shot flag.
        timer.since = if timer.once { None } else { Some(now) };
        self.reschedule();
        Some(&self.timers[index].kind)
    }

    fn reschedule(&mut self) {
        self.next = self.timers.iter().filter_map(|t| self.due(t)).min();
    }

    /// `ctl status`: what LG's idle timers are and when each is due (a list that is empty on a set whose Screen Saver timer is on says the
    /// subscription never delivered). `intervalSec` is LG's, `waitSec` the one run (null: Off).
    pub fn report(&self, now: Instant) -> Value {
        let timers: Vec<Value> = self.timers.iter().map(|t| serde_json::json!({
            "type": t.kind, "intervalSec": t.interval.as_secs(), "waitSec": self.wait(t).map(|w| w.as_secs()),
            "once": t.once, "repeat": t.repeat,
            "dueInSec": self.due(t).map(|d| d.saturating_duration_since(now).as_secs()),
        })).collect();
        serde_json::json!({"subscribed": self.token.is_some(), "timers": timers})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn timer(kind: &str, seconds: u64, once: bool, repeat: bool) -> Value {
        json!({"type": kind, "interval": seconds, "once": once, "repeat": repeat})
    }
    fn update(timers: &mut Timers, values: Vec<Value>, now: Instant) {
        timers.update(&json!({"returnValue": true, "timers": values}), now);
    }

    #[test]
    fn snapshots_do_not_postpone_existing_idle_deadlines() {
        let start = Instant::now();
        let mut timers = Timers::default();
        let saver = timer("saver", 180, false, true);
        update(&mut timers, vec![saver.clone()], start);
        update(&mut timers, vec![saver, timer("other", 30, false, false)], start + Duration::from_secs(100));
        assert_eq!(timers.expired(start + Duration::from_secs(130)), Some("other"));
        assert_eq!(timers.expired(start + Duration::from_secs(180)), Some("saver"));
    }

    #[test]
    fn input_and_daemon_remove_readd_restart_the_idle_countdown() {
        let start = Instant::now();
        let mut timers = Timers::default();
        let saver = timer("saver", 180, false, true);
        update(&mut timers, vec![saver.clone()], start);
        timers.activity(start + Duration::from_secs(150));
        assert_eq!(timers.expired(start + Duration::from_secs(180)), None);
        assert_eq!(timers.deadline(), Some(start + Duration::from_secs(330)));
        update(&mut timers, vec![], start + Duration::from_secs(200));
        update(&mut timers, vec![saver], start + Duration::from_secs(200));
        assert_eq!(timers.expired(start + Duration::from_secs(330)), None);
        assert_eq!(timers.expired(start + Duration::from_secs(380)), Some("saver"));
    }

    #[test]
    fn once_is_single_shot_until_daemon_rearms_it() {
        let start = Instant::now();
        let mut timers = Timers::default();
        let one = timer("once", 10, true, true);
        update(&mut timers, vec![one.clone()], start);
        assert_eq!(timers.expired(start + Duration::from_secs(10)), Some("once"));
        timers.activity(start + Duration::from_secs(11));
        update(&mut timers, vec![one.clone()], start + Duration::from_secs(12));
        assert_eq!(timers.deadline(), None);
        update(&mut timers, vec![], start + Duration::from_secs(13));
        update(&mut timers, vec![one], start + Duration::from_secs(13));
        assert_eq!(timers.expired(start + Duration::from_secs(23)), Some("once"));
    }

    #[test]
    fn daemon_repeat_flag_does_not_make_a_local_single_shot() {
        let start = Instant::now();
        let mut timers = Timers::default();
        update(&mut timers, vec![timer("idle", 10, false, false)], start);
        assert_eq!(timers.expired(start + Duration::from_secs(10)), Some("idle"));
        assert_eq!(timers.expired(start + Duration::from_secs(10)), None);
        assert_eq!(timers.expired(start + Duration::from_secs(20)), Some("idle"));
        update(&mut timers, vec![], start + Duration::from_secs(21));
        assert_eq!(timers.expired(start + Duration::from_secs(100)), None);
    }

    #[test]
    fn timeout_change_cancels_old_deadline_and_overdue_tick_does_not_burst() {
        let start = Instant::now();
        let mut timers = Timers::default();
        update(&mut timers, vec![timer("saver", 180, false, true)], start);
        update(&mut timers, vec![timer("saver", 300, false, true)], start + Duration::from_secs(100));
        assert_eq!(timers.expired(start + Duration::from_secs(180)), None);
        assert_eq!(timers.expired(start + Duration::from_secs(1000)), Some("saver"));
        assert_eq!(timers.expired(start + Duration::from_secs(1000)), None);
        assert_eq!(timers.deadline(), Some(start + Duration::from_secs(1300)));
    }

    #[test]
    fn the_screen_saver_timer_runs_at_lumatvs_wait_and_every_other_at_lgs() {
        let start = Instant::now();
        let at = |s| start + Duration::from_secs(s);
        let mut timers = Timers::default();
        update(&mut timers, vec![timer("[TIMER]ScreenSaver:180", 180, false, true), timer("[TIMER]#1", 3600, false, false)], start);
        assert_eq!(timers.expired(at(180)), None, "never at LG's 3 minutes");
        assert_eq!(timers.expired(at(300)), Some("[TIMER]ScreenSaver:180"), "the shipped 5");
        timers.set_saver_after(Some(Duration::from_secs(60)));
        assert_eq!(timers.deadline(), Some(at(360)), "a new choice applies to the count running");
        timers.set_saver_after(None);
        assert_eq!(timers.deadline(), Some(at(3600)), "Off: the saver's timer never ends; LG's other keeps its own interval");
        let report = timers.report(at(3000));
        assert_eq!(report["timers"][0], json!({"type": "[TIMER]ScreenSaver:180", "intervalSec": 180, "waitSec": null, "once": false, "repeat": true, "dueInSec": null}));
        assert_eq!(timers.expired(at(3600)), Some("[TIMER]#1"));
    }
}
