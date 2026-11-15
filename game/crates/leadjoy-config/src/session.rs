//! A caller-owned transport boundary: constructing a controller never opens a device.

use serde_json::Value;
use std::{
    io,
    time::{Duration, Instant},
};

use crate::{
    Error, Result,
    command::{Change, Command, LedFrame, Request, Response},
    frame::{Decoder, REPORT_LEN, Report},
};

/// Transfers the vendor's 64-byte endpoint payload, without a hidraw report-id prefix.
/// An adapter must select the correct vendor interface and honour the remaining timeout.
/// One session must own the endpoint exclusively; the protocol has no transaction id.
pub trait Transport {
    fn send(&mut self, report: &Report, timeout: Duration) -> io::Result<()>;
    fn recv(&mut self, report: &mut Report, timeout: Duration) -> io::Result<Option<usize>>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunMode {
    #[default]
    DryRun,
    Apply,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Execution {
    pub write_reports: Vec<Report>,
    pub read_reports: Vec<Report>,
    /// Present only after an acknowledged write and successful comparison against a fresh read.
    pub read_back: Option<Response>,
}

/// Operations such as calibration and recording have an ACK/status, not a setting comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandReceipt {
    pub reports: Vec<Report>,
    pub response: Option<Response>,
}

/// Builds both sides of a write/read-back plan without constructing or opening a transport.
pub fn dry_run(change: &Change) -> Result<Execution> {
    if change.write.is_read() || !change.read.is_read() || change.expected.is_null() || change.expected.as_object().is_some_and(|o| o.is_empty()) {
        return Err(Error::Invalid("change requires a write, a query, and non-empty read-back expectations"));
    }
    Ok(Execution { write_reports: change.write.reports()?, read_reports: change.read.reports()?, read_back: None })
}

pub struct Controller<T> {
    transport: T,
    timeout: Duration,
}

impl<T: Transport> Controller<T> {
    pub fn new(transport: T, timeout: Duration) -> Self {
        Self { transport, timeout }
    }
    pub fn into_transport(self) -> T {
        self.transport
    }

    /// Only non-mutating requests are allowed through this convenience entry point.
    pub fn read(&mut self, request: &Request) -> Result<Response> {
        if !request.is_read() {
            return Err(Error::Invalid("read requires a non-mutating request"));
        }
        self.exchange(request.command(), &request.reports()?)
    }

    /// DryRun performs no sends or receives. Apply writes once, requires an acknowledgement,
    /// then reads the setting back; a failed or timed-out write is never automatically retried.
    /// A read-back failure after the acknowledgement is `Error::Unverified`, not a failed write.
    pub fn apply(&mut self, change: &Change, mode: RunMode) -> Result<Execution> {
        let mut execution = dry_run(change)?;
        if mode == RunMode::Apply {
            self.exchange(change.write.command(), &execution.write_reports)?;
            let reply = self.exchange(change.read.command(), &execution.read_reports).map_err(|error| Error::Unverified(Box::new(error)))?;
            if !matches_expected(&reply.result, &change.expected) {
                return Err(Error::ReadBack);
            }
            execution.read_back = Some(reply);
        }
        Ok(execution)
    }

    /// Explicitly executes a non-query operation. This does not claim setting read-back success;
    /// ordinary settings should use `apply` instead.
    pub fn execute(&mut self, request: &Request, mode: RunMode) -> Result<CommandReceipt> {
        if request.is_read() {
            return Err(Error::Invalid("queries use read, not execute"));
        }
        let reports = request.reports()?;
        let response = if mode == RunMode::Apply { Some(self.exchange(request.command(), &reports)?) } else { None };
        Ok(CommandReceipt { reports, response })
    }

    /// Waits for a matching unsolicited report (for example c175 macro steps), without sending.
    pub fn receive(&mut self, command: Command) -> Result<Response> {
        self.exchange(command, &[])
    }

    fn exchange(&mut self, command: Command, reports: &[Report]) -> Result<Response> {
        let deadline = Instant::now().checked_add(self.timeout).ok_or(Error::Invalid("timeout overflow"))?;
        for report in reports {
            self.transport.send(report, remaining(deadline)?)?;
        }
        let mut decoder = Decoder::default();
        let mut report = [0; REPORT_LEN];
        loop {
            let count = self.transport.recv(&mut report, remaining(deadline)?)?.ok_or(Error::NoReply)?;
            if count > REPORT_LEN {
                return Err(Error::Invalid("transport returned an oversized report"));
            }
            if let Some(payload) = decoder.push(&report[..count])? {
                let response = Response::decode(&payload)?;
                if response.command == command as u8 {
                    response.ensure_success()?;
                    return Ok(response);
                }
            }
        }
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(Error::NoReply)
}

fn matches_expected(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            if e.contains_key("brt") && e.contains_key("m") && e.contains_key("s") && e.contains_key("rgb") {
                if let (Ok(a), Ok(e)) = (LedFrame::from_value(actual), LedFrame::from_value(expected)) {
                    return a == e;
                }
                return false;
            }
            e.iter().all(|(key, value)| a.get(key).is_some_and(|actual| matches_expected(actual, value)))
        }
        (Value::Array(a), Value::Array(e)) => a.len() == e.len() && a.iter().zip(e).all(|(a, e)| matches_expected(a, e)),
        _ => actual == expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Lighting;
    use serde_json::json;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Script {
        replies: VecDeque<Report>,
        sent: Vec<Report>,
        reads: usize,
    }

    impl Script {
        fn reply(&mut self, command: u8, error: u8, result: Value) {
            self.replies.extend(crate::encode(&json!({"c": command, "e": error, "r": result})).unwrap());
        }
    }

    impl Transport for Script {
        fn send(&mut self, report: &Report, _: Duration) -> io::Result<()> {
            self.sent.push(*report);
            Ok(())
        }
        fn recv(&mut self, report: &mut Report, _: Duration) -> io::Result<Option<usize>> {
            self.reads += 1;
            Ok(self.replies.pop_front().map(|r| {
                *report = r;
                REPORT_LEN
            }))
        }
    }

    fn change() -> Change {
        Change::lighting(Lighting { brightness: 80, rgb: [10, 20, 30] })
    }

    #[test]
    fn dry_run_never_touches_transport() {
        let mut controller = Controller::new(Script::default(), Duration::ZERO);
        let execution = controller.apply(&change(), RunMode::default()).unwrap();
        assert_eq!(execution.write_reports.len(), 1);
        assert_eq!(execution.read_reports.len(), 1);
        assert!(execution.read_back.is_none());
        let script = controller.into_transport();
        assert!(script.sent.is_empty());
        assert_eq!(script.reads, 0);
    }

    #[test]
    fn apply_waits_for_ack_then_checks_fresh_read_back() {
        let change = change();
        let mut script = Script::default();
        script.reply(14, 0, json!(99)); // Unrelated input/status responses cannot acknowledge a write.
        script.reply(190, 0, Value::Null);
        script.reply(190, 0, change.expected.clone());
        let mut controller = Controller::new(script, Duration::from_secs(1));
        let execution = controller.apply(&change, RunMode::Apply).unwrap();
        assert_eq!(execution.read_back.unwrap().result, change.expected);
        let script = controller.into_transport();
        assert_eq!(script.sent, [change.write.reports().unwrap(), change.read.reports().unwrap()].concat());
    }

    #[test]
    fn mismatch_device_error_and_timeout_are_not_success() {
        let mut script = Script::default();
        script.reply(190, 0, Value::Null);
        script.reply(190, 0, json!({"brt":79,"rgb":[10,20,30]}));
        let mut controller = Controller::new(script, Duration::from_secs(1));
        assert!(matches!(controller.apply(&change(), RunMode::Apply), Err(Error::ReadBack)));

        let mut script = Script::default();
        script.reply(190, 4, Value::Null);
        let mut controller = Controller::new(script, Duration::from_secs(1));
        assert!(matches!(controller.apply(&change(), RunMode::Apply), Err(Error::Device(4))));
        assert_eq!(controller.into_transport().sent.len(), 1);

        let mut script = Script::default();
        script.reply(190, 0, Value::Null);
        let mut controller = Controller::new(script, Duration::from_secs(1));
        let unverified = controller.apply(&change(), RunMode::Apply);
        assert!(matches!(unverified, Err(Error::Unverified(error)) if matches!(*error, Error::NoReply)));
        assert_eq!(controller.into_transport().sent.len(), 2);

        let mut controller = Controller::new(Script::default(), Duration::from_secs(1));
        assert!(matches!(controller.read(&Request::query(Command::DeviceVersion)), Err(Error::NoReply)));
        assert_eq!(controller.into_transport().sent.len(), 1);
        let mut controller = Controller::new(Script::default(), Duration::ZERO);
        assert!(matches!(controller.apply(&change(), RunMode::Apply), Err(Error::NoReply)));
        assert!(controller.into_transport().sent.is_empty());
    }

    #[test]
    fn only_read_queries_and_complete_expectations_are_accepted() {
        let mut controller = Controller::new(Script::default(), Duration::ZERO);
        assert!(controller.read(&change().write).is_err());
        let mut invalid = change();
        invalid.expected = json!({});
        assert!(controller.apply(&invalid, RunMode::DryRun).is_err());
        assert!(controller.into_transport().sent.is_empty());
        assert!(matches_expected(&json!({"brt":80,"m":2,"s":500,"rgb":[[1,2,3]],"extra":true}), &json!({"brt":80,"m":2,"s":500,"rgb":[1,2,3]})));
        assert!(!matches_expected(&json!({"rgb":[1,2]}), &json!({"rgb":[1,2,3]})));
    }
}
