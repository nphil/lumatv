//! The conversation with the app: requests as lines on stdin, events as lines on stdout. Only this thread writes the output.

use crate::http::Http;
use crate::{Adapter, Clock};
use lumagame_core::ipc::{self, ErrorCode, Event, FromEngine, LineBuffer, ToEngine};
use serde_json::Value;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

enum Input {
    Line(String),
    /// A line over the limit or not text. The stream cannot be trusted after it, so the engine ends.
    Invalid,
    Eof,
}

/// Cuts stdin into lines on a thread of its own, so that the engine can wait for a line and for the next check of a sign-in at once.
/// The stream ends at its first bad line: nothing after it is passed on, even when it came in the same read.
fn reader(mut input: impl Read, tx: SyncSender<Input>) {
    let mut lines = LineBuffer::default();
    let mut bytes = [0u8; 8192];
    loop {
        match input.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                let mut rest = &bytes[..n];
                while !rest.is_empty() {
                    let end = rest.iter().position(|b| *b == b'\n').map_or(rest.len(), |at| at + 1);
                    let (piece, tail) = rest.split_at(end);
                    rest = tail;
                    let before = lines.dropped;
                    let mut gone = false;
                    lines.push(piece, |line| gone = tx.send(Input::Line(line.to_owned())).is_err());
                    if gone {
                        return;
                    }
                    if lines.dropped != before {
                        let _ = tx.send(Input::Invalid);
                        return;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = tx.send(Input::Eof);
}

fn refusal(re: Option<u64>, code: ErrorCode, message: &str) -> FromEngine {
    FromEngine { re, event: Event::Error { code, message: message.to_owned() } }
}

/// The answer to a line that is not a request this build knows. It names the request's seq when the line has one.
fn unreadable(line: &str) -> FromEngine {
    let seq = serde_json::from_str::<Value>(line).ok().and_then(|value| value["seq"].as_u64());
    refusal(seq, ErrorCode::Unsupported, "Unknown or malformed engine request")
}

fn send(output: &mut impl Write, events: &[FromEngine]) -> io::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    for event in events {
        output.write_all(ipc::encode(event).as_bytes())?;
    }
    output.flush()
}

/// Runs until the app closes stdin or the output, or the engine ends the conversation itself.
pub fn run<H: Http, C: Clock>(mut adapter: Adapter<H, C>, input: impl Read + Send + 'static, mut output: impl Write) -> io::Result<()> {
    let (tx, rx) = mpsc::sync_channel(64);
    thread::Builder::new().name("steam-reader".into()).spawn(move || reader(input, tx))?;
    match serve(&mut adapter, &rx, &mut output) {
        Err(error) if matches!(error.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) => Ok(()),
        result => result,
    }
}

fn serve<H: Http, C: Clock>(adapter: &mut Adapter<H, C>, rx: &Receiver<Input>, output: &mut impl Write) -> io::Result<()> {
    while !adapter.finished() {
        let next = match adapter.next_wake() {
            Some(ms) => rx.recv_timeout(Duration::from_millis(ms)),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let mut events = match next {
            Ok(Input::Line(line)) => match ipc::decode::<ToEngine>(&line) {
                Ok(ToEngine { seq, request }) => adapter.handle(seq, request),
                Err(_) => vec![unreadable(&line)],
            },
            Ok(Input::Invalid) => return send(output, &[refusal(None, ErrorCode::BadRequest, "The protocol line is invalid or too long")]),
            Ok(Input::Eof) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => Vec::new(),
        };
        events.extend(adapter.tick());
        send(output, &events)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeClock, FakeHttp, Rig};
    use lumagame_core::engine::AuthMethod;
    use lumagame_core::ipc::{AuthState, Request};
    use std::io::Cursor;

    fn adapter(rig: &Rig) -> Adapter<FakeHttp, FakeClock> {
        Adapter::new(rig.http.clone(), rig.clock.clone(), rig.log.sink())
    }

    fn line(seq: u64, request: Request) -> String {
        ipc::encode(&ToEngine { seq, request })
    }

    /// Runs the engine over `input` to its end and returns what it wrote.
    fn converse(rig: &Rig, input: &str) -> (io::Result<()>, Vec<FromEngine>) {
        let mut output = Vec::new();
        let result = run(adapter(rig), Cursor::new(input.as_bytes().to_vec()), &mut output);
        let events = String::from_utf8(output).unwrap().lines().map(|line| ipc::decode(line).unwrap()).collect();
        (result, events)
    }

    fn hello(rig: &Rig) -> String {
        line(1, Request::Hello(rig.hello_request()))
    }

    #[test]
    fn requests_are_answered_in_order_and_the_run_ends_when_the_app_closes_its_side() {
        let rig = Rig::new();
        let (result, events) = converse(&rig, &(hello(&rig) + &line(2, Request::Targets)));
        assert!(result.is_ok());
        let [FromEngine { re: Some(1), event: Event::Ready { .. } }, FromEngine { re: Some(2), event: Event::Targets { targets } }] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert!(targets.is_empty());
    }

    #[test]
    fn a_line_the_engine_cannot_read_is_answered_with_its_seq_and_the_conversation_goes_on() {
        let rig = Rig::new();
        let garbage = "{\"seq\":5,\"type\":\"nonsense\"}\nnot json at all\n{\"type\":\"targets\"}\n[]\n";
        let (result, events) = converse(&rig, &(hello(&rig) + garbage + &line(2, Request::Targets)));
        assert!(result.is_ok());
        let error = |re| FromEngine { re, event: Event::Error { code: ErrorCode::Unsupported, message: "Unknown or malformed engine request".to_owned() } };
        assert_eq!(&events[1..5], [error(Some(5)), error(None), error(None), error(None)]);
        assert!(matches!(events[5], FromEngine { re: Some(2), event: Event::Targets { .. } }));
        assert_eq!(events.len(), 6);
    }

    #[test]
    fn a_hello_for_another_protocol_ends_the_run_with_nothing_after_the_error() {
        let rig = Rig::new();
        let mut other = rig.hello_request();
        other.protocol += 1;
        let (result, events) = converse(&rig, &(line(1, Request::Hello(other)) + &line(2, Request::Targets)));
        assert!(result.is_ok());
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(matches!(&events[0], FromEngine { re: Some(1), event: Event::Error { code: ErrorCode::Protocol, .. } }));
    }

    #[test]
    fn a_line_over_the_limit_ends_the_run_with_an_error() {
        let rig = Rig::new();
        let huge = "x".repeat(ipc::MAX_LINE_BYTES + 1) + "\n";
        let (result, events) = converse(&rig, &(hello(&rig) + &huge + &line(2, Request::Targets)));
        assert!(result.is_ok());
        assert_eq!(events.len(), 2, "{events:?}");
        let FromEngine { re: None, event: Event::Error { code: ErrorCode::BadRequest, message } } = &events[1] else { panic!("{events:?}") };
        assert_eq!(message, "The protocol line is invalid or too long");
    }

    #[test]
    fn a_sign_in_in_progress_does_not_hold_the_engine_back_from_ending() {
        let rig = Rig::new();
        let begin = r#"{"response":{"client_id":"123","challenge_url":"https://s.team/q/1/123","request_id":"AAAA","interval":5}}"#;
        rig.http.on("BeginAuthSessionViaQR", [crate::fake::ok(begin)]);
        let (result, events) = converse(&rig, &(hello(&rig) + &line(2, Request::Auth { target: None, method: AuthMethod::Qr })));
        assert!(result.is_ok());
        assert_eq!(events.len(), 2, "{events:?}");
        let FromEngine { re: Some(2), event: Event::Auth { state: AuthState::Qr { url, expires_s: 300 } } } = &events[1] else { panic!("{events:?}") };
        assert_eq!(url, "https://s.team/q/1/123");
    }

    #[test]
    fn a_closed_output_ends_the_run_quietly() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let rig = Rig::new();
        let input = hello(&rig).into_bytes();
        assert!(run(adapter(&rig), Cursor::new(input), Closed).is_ok());
    }
}
