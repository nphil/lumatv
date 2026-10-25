//! The receiver as the AI module acts on it: volume and mute through the Denon's own web interface.
//!
//! The address and the on/off switch are read from the `base.receiver` settings at every call, so a change made on the TV
//! applies to the next command. Nothing here can reach any other host, and nothing an assistant passes reaches the request
//! line except through `volume_command` and `mute_command`.

use crate::ai::types::{ActionError, ReceiverControl};
use crate::config::ReceiverTuning;
use crate::settings::SettingsStore;
use lumatv_denon::{Client, ClientError, mute_command, volume_command};
use std::sync::Arc;
use std::time::Duration;

/// Sends volume and mute commands to the receiver named in the settings.
pub struct DenonControl {
    settings: Arc<SettingsStore>,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl DenonControl {
    pub fn new(settings: Arc<SettingsStore>, tuning: ReceiverTuning) -> Self {
        Self {
            settings,
            connect_timeout: Duration::from_millis(tuning.connect_timeout_ms),
            request_timeout: Duration::from_millis(tuning.request_timeout_ms),
        }
    }

    /// Sends one command to the receiver the settings name now.
    fn send(&self, command: &str) -> Result<(), ActionError> {
        let receiver = self.settings.receiver();
        if !receiver.enabled {
            return Err(ActionError::new(
                "unavailable",
                "the receiver is switched off in the TV's settings",
            ));
        }
        Client::new(receiver.host, receiver.port)
            .with_timeouts(self.connect_timeout, self.request_timeout)
            .send_command(command)
            .map_err(|e| action_error(&e))
    }
}

/// A client error as the assistant is told it: one plain line that names neither the host nor the port (the client's own
/// texts can carry resolver details).
fn action_error(e: &ClientError) -> ActionError {
    match e {
        ClientError::Unreachable(_) | ClientError::Io(_) => {
            ActionError::new("unavailable", "the receiver cannot be reached")
        }
        ClientError::Timeout => ActionError::new("timeout", "the receiver did not answer in time"),
        ClientError::Http(_) => ActionError::new("failed", "the receiver refused the command"),
        ClientError::BadResponse(_) | ClientError::TooLarge | ClientError::Parse(_) => {
            ActionError::new("failed", "the receiver's reply was not usable")
        }
        ClientError::InvalidCommand => {
            ActionError::new("refused", "the command is not a valid receiver command")
        }
    }
}

impl ReceiverControl for DenonControl {
    fn set_volume(&self, level: f64) -> Result<(), ActionError> {
        let command = volume_command(level)
            .ok_or_else(|| ActionError::new("refused", "the volume must be between 0 and 98"))?;
        self.send(&command)
    }

    fn set_mute(&self, muted: bool) -> Result<(), ActionError> {
        self.send(mute_command(muted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    /// A receiver that answers `count` connections with `reply` and yields each request line.
    fn fake_receiver(reply: &'static str, count: usize) -> (u16, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let mut lines = Vec::new();
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 512];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                }
                let _ = stream.write_all(reply.as_bytes());
                let text = String::from_utf8_lossy(&request).into_owned();
                lines.push(text.lines().next().unwrap_or("").to_owned());
            }
            lines
        });
        (port, handle)
    }

    fn control(dir: &std::path::Path, enabled: bool, port: u16) -> DenonControl {
        let settings_dir = dir.join("settings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let doc = json!({"owner": "base.receiver", "schemaRevision": 1, "version": 1, "values": {
            "enabled": {"type": "bool", "value": enabled},
            "host": {"type": "text", "value": "127.0.0.1"},
            "port": {"type": "integer", "value": port}
        }});
        std::fs::write(settings_dir.join("base.receiver.json"), doc.to_string()).unwrap();
        let store = SettingsStore::new(&settings_dir, Arc::new(Hub::new(1)));
        store.register_builtin().unwrap();
        let tuning = ReceiverTuning {
            connect_timeout_ms: 500,
            request_timeout_ms: 1000,
            ..ReceiverTuning::default()
        };
        DenonControl::new(Arc::new(store), tuning)
    }

    #[test]
    fn volume_and_mute_reach_the_receiver_as_commands() {
        let dir = tempfile::tempdir().unwrap();
        let (port, requests) = fake_receiver("HTTP/1.0 200 OK\r\n\r\n", 3);
        let ctl = control(dir.path(), true, port);
        ctl.set_volume(35.5).unwrap();
        ctl.set_volume(5.0).unwrap();
        ctl.set_mute(true).unwrap();
        assert_eq!(
            requests.join().unwrap(),
            [
                "GET /goform/formiPhoneAppDirect.xml?MV355 HTTP/1.0",
                "GET /goform/formiPhoneAppDirect.xml?MV05 HTTP/1.0",
                "GET /goform/formiPhoneAppDirect.xml?MUON HTTP/1.0",
            ]
        );
    }

    #[test]
    fn a_level_outside_the_scale_is_refused_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let ctl = control(dir.path(), true, listener.local_addr().unwrap().port());
        for level in [-1.0, 98.5, f64::NAN, f64::INFINITY] {
            assert_eq!(ctl.set_volume(level).unwrap_err().code, "refused");
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn a_receiver_switched_off_in_the_settings_is_unavailable_and_not_contacted() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let ctl = control(dir.path(), false, listener.local_addr().unwrap().port());
        let e = ctl.set_mute(false).unwrap_err();
        assert_eq!(e.code, "unavailable");
        assert_eq!(
            e.message,
            "the receiver is switched off in the TV's settings"
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn a_receiver_that_refuses_is_a_failure_that_names_no_address() {
        let dir = tempfile::tempdir().unwrap();
        let (port, _) = fake_receiver("HTTP/1.0 403 Forbidden\r\n\r\n", 1);
        let e = control(dir.path(), true, port)
            .set_volume(20.0)
            .unwrap_err();
        assert_eq!(e.code, "failed");
        assert!(!e.message.contains("127.0.0.1") && !e.message.contains(&port.to_string()));
    }

    #[test]
    fn a_closed_port_is_unavailable_and_a_silent_receiver_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let e = control(dir.path(), true, closed)
            .set_mute(true)
            .unwrap_err();
        assert_eq!(e.code, "unavailable");
        assert!(!e.message.contains("127.0.0.1") && !e.message.contains(&closed.to_string()));

        let silent = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = silent.local_addr().unwrap().port();
        let hold = thread::spawn(move || {
            let (_stream, _) = silent.accept().unwrap();
            thread::sleep(Duration::from_millis(1500));
        });
        let dir2 = tempfile::tempdir().unwrap();
        let e = control(dir2.path(), true, port).set_mute(true).unwrap_err();
        assert_eq!(e.code, "timeout");
        drop(hold);
    }

    #[test]
    fn client_errors_map_to_the_documented_codes() {
        for (error, code) in [
            (ClientError::Unreachable("x".into()), "unavailable"),
            (ClientError::Timeout, "timeout"),
            (ClientError::Io("x".into()), "unavailable"),
            (ClientError::Http(500), "failed"),
            (ClientError::BadResponse("x"), "failed"),
            (ClientError::TooLarge, "failed"),
            (ClientError::InvalidCommand, "refused"),
        ] {
            assert_eq!(action_error(&error).code, code, "{error:?}");
        }
    }
}
