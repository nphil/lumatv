//! LG's NoOp timers are executed by the compositor, not by nopd itself.

use std::time::{Duration, Instant};

use serde_json::Value;

pub(crate) const TIMER_INFO: &str = "luna://com.webos.service.nop/getTimerInfo";
pub(crate) const TIMER_EXPIRED: &str = "luna://com.webos.service.nop/notifyExpireTimer";

struct Timer {
    kind: String,
    interval: Duration,
    once: bool,
    repeat: bool,
    due: Option<Instant>,
    seen: bool,
}

#[derive(Default)]
pub(crate) struct Timers {
    pub token: Option<u64>,
    timers: Vec<Timer>,
    next: Option<Instant>,
}

impl Timers {
    pub fn deadline(&self) -> Option<Instant> { self.next }

    pub fn update(&mut self, payload: &Value, now: Instant) {
        let Some(timers) = payload.get("timers").and_then(Value::as_array) else { return; };
        for timer in &mut self.timers { timer.seen = false; }
        for value in timers {
            let Some(kind) = value.get("type").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue; };
            let Some(seconds) = value.get("interval").and_then(Value::as_u64).filter(|n| *n > 0) else { continue; };
            let Some(once) = value.get("once").and_then(Value::as_bool) else { continue; };
            let Some(repeat) = value.get("repeat").and_then(Value::as_bool) else { continue; };
            let interval = Duration::from_secs(seconds);
            let Some(due) = now.checked_add(interval) else { continue; };
            // TimerMgr publishes complete lists, including unchanged timers. Its reset is a
            // remove/readd; resetting every snapshot would indefinitely postpone other timers.
            let existing = self.timers.iter().position(|t| t.kind == kind && t.interval == interval
                && t.once == once && t.repeat == repeat);
            if let Some(index) = existing {
                self.timers[index].seen = true;
            } else {
                self.timers.push(Timer { kind: kind.into(), interval, once, repeat, due: Some(due), seen: true });
            }
        }
        self.timers.retain(|timer| timer.seen);
        self.reschedule();
    }

    pub fn activity(&mut self, now: Instant) {
        for timer in &mut self.timers {
            if timer.due.is_some() { timer.due = now.checked_add(timer.interval); }
        }
        self.reschedule();
    }

    pub fn expired(&mut self, now: Instant) -> Option<&str> {
        if self.next.is_none_or(|due| due > now) { return None; }
        let index = self.timers.iter().position(|t| t.due.is_some_and(|due| due <= now))?;
        let timer = &mut self.timers[index];
        // `once` controls LG's local QTimer. `repeat` instead controls nopd's
        // ownership/removal after the notification; it is not the single-shot flag.
        timer.due = if timer.once { None } else { now.checked_add(timer.interval) };
        self.reschedule();
        Some(&self.timers[index].kind)
    }

    fn reschedule(&mut self) {
        self.next = self.timers.iter().filter_map(|t| t.due).min();
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
}
