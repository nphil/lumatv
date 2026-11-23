//! The processes behind a running app, and what can be told of them from `/proc`.
//!
//! The app manager lists each running app with the pid of its process (`processid`; web apps also have `webprocessid`, which is the same
//! process). A pid alone is not a name: once the process ends the system may give the number to another one, and a kill sent later would
//! hit a stranger. So a process is held as its pid and the time it started (field 22 of `/proc/PID/stat`, in clock ticks after boot), and
//! every signal re-reads that time first.

use std::fs;

use serde_json::Value;

/// A process: pid and start time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub start: u64,
}

/// A process's state letter and start time, from the text of `/proc/PID/stat`. The command name sits in parentheses and may hold spaces and
/// parentheses of its own, so the fields are counted from the last `)`.
pub fn parse_stat(text: &str) -> Option<(char, u64)> {
    let rest = &text[text.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    // after the state: ppid (field 4) to itrealvalue (21), then starttime (22)
    let start = fields.nth(18)?.parse().ok()?;
    Some((state, start))
}

/// A pid as the app manager writes it: a string of digits (empty for none) or a number. Pid 1 and below are never an app's.
pub fn parse_pid(v: &Value) -> Option<i32> {
    let pid = match v {
        Value::String(s) => s.trim().parse::<i32>().ok()?,
        Value::Number(n) => i32::try_from(n.as_i64()?).ok()?,
        _ => return None,
    };
    (pid > 1).then_some(pid)
}

impl Proc {
    /// The process with this pid, as it is now; `None` when there is none.
    pub fn identify(pid: i32) -> Option<Proc> {
        let (_, start) = parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
        Some(Proc { pid, start })
    }

    /// Its state letter now; `None` when it has ended or its pid belongs to another process.
    fn state(&self) -> Option<char> {
        let (state, start) = parse_stat(&fs::read_to_string(format!("/proc/{}/stat", self.pid)).ok()?)?;
        (start == self.start).then_some(state)
    }

    /// Stopped (`SIGSTOP`, or held by a tracer): it will never act on a request to quit, however long it is given.
    pub fn stopped(&self) -> bool {
        matches!(self.state(), Some('T' | 't'))
    }

    /// Whether it is still the same process, and not ended.
    pub fn alive(&self) -> bool {
        self.state().is_some_and(|s| s != 'Z' && s != 'X')
    }

    /// `SIGKILL`, if it is still the process it was. Whether the signal was sent.
    pub fn kill(&self) -> bool {
        // SAFETY: `kill(2)` with a pid above 1 that was just checked to be the process this value names; no memory is involved.
        self.alive() && unsafe { libc::kill(self.pid, libc::SIGKILL) } == 0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_state_and_start_come_from_after_the_last_parenthesis() {
        let line = "9847 (WebAppMgr) S 1 9847 9847 0 -1 4194560 1 2 3 4 5 6 7 8 20 0 12 0 31337 100 200 18446744073709551615";
        assert_eq!(parse_stat(line), Some(('S', 31337)));
        // a command name with spaces and a closing parenthesis in it
        let odd = "12 (a) b (c)) T 1 12 12 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 555 1 1 1";
        assert_eq!(parse_stat(odd), Some(('T', 555)));
    }

    #[test]
    fn a_line_that_is_cut_short_is_not_a_process() {
        assert_eq!(parse_stat("12 (a) S 1 12"), None);
        assert_eq!(parse_stat("no parenthesis"), None);
        assert_eq!(parse_stat(""), None);
    }

    #[test]
    fn pids_are_digits_or_numbers_and_one_or_less_is_none() {
        assert_eq!(parse_pid(&json!("3754")), Some(3754));
        assert_eq!(parse_pid(&json!(3754)), Some(3754));
        assert_eq!(parse_pid(&json!("")), None);
        assert_eq!(parse_pid(&json!("1")), None);
        assert_eq!(parse_pid(&json!("-5")), None);
        assert_eq!(parse_pid(&json!(null)), None);
        assert_eq!(parse_pid(&json!("not a pid")), None);
    }

    #[test]
    fn this_process_is_alive_and_not_stopped_and_a_wrong_start_names_nobody() {
        let me = Proc::identify(std::process::id() as i32).expect("/proc/self is readable");
        assert!(me.alive());
        assert!(!me.stopped());
        let stranger = Proc { start: me.start + 1, ..me };
        assert!(!stranger.alive());
        assert!(!stranger.kill(), "a pid with another start time is not signalled");
    }
}
