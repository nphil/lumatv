//! The receiver's telnet port (23): one held connection that pushes every change.
//!
//! Measured on the set's S760H (`docs/research/native-helper/denon2-2026-10-25-1648/`): it accepts a second client beside
//! Home Assistant's, answers `PW?` with `PWON`, `MV?` with `MV355` (35.5 on the panel's 0 to 98 scale) and `MVMAX 665`,
//! `MU?` with `MUOFF`, `SI?` with `SITV` and `SVOFF`, ends every line with `\r`, and pushes each change (`MV36`) to every
//! client. This module only reads: [`Session::query`] refuses anything that is not a query. Commands go over HTTP
//! ([`crate::Client::send_command`]); their effect comes back here as a push.
//!
//! [`parse_line`] and [`Snapshot`] are pure; [`Session`] is the blocking transport with no thread of its own.

use crate::client::{Client, ClientError, io_error, short};
use crate::status::{Power, Status, VolumeDisplay};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

/// The default telnet port.
pub const DEFAULT_TELNET_PORT: u16 = 23;

/// The four queries that together say everything a [`Status`] holds.
pub const QUERIES: [&str; 4] = ["PW?", "MV?", "MU?", "SI?"];

/// Two more read-only questions: the sound mode (`MSDOLBY AUDIO-DSUR`) and the input signal in words (`SSINFAISSIG 13` and `SYSDA PCM ZERO`). They are
/// not part of a [`Status`]; the caller keeps what they answer beside it and publishes it raw.
pub const FORMAT_QUERIES: [&str; 2] = ["MS?", "SSINFAISSIG ?"];

/// The longest line kept while waiting for its end; the receiver's lines are a dozen characters.
const MAX_LINE: usize = 4096;

/// One thing the receiver said about its main zone.
#[derive(Clone, Debug, PartialEq)]
pub enum Push {
    Power(Power),
    /// `MasterVolume` in dB relative to the reference level: the panel's 0 to 98 scale minus 80, as the status XML writes it.
    Volume(f64),
    Mute(bool),
    /// The input as the receiver names it (`TV`, `SAT/CBL`, ...), the same text as `InputFuncSelect`.
    Input(String),
    /// The sound mode as the receiver names it (`MSDOLBY AUDIO-DSUR`, `MSSTEREO`), without the `MS`.
    Mode(String),
    /// The input signal in the receiver's words (`SYSDA PCM ZERO`), without the `SYSDA` and the padding after it.
    Signal(String),
    /// The receiver's volume limit on the panel's 0 to 98 scale (`MVMAX 665` is 66.5): the loudest an `MV` command can set.
    MaxVolume(f64),
}

/// What a line means, or `None` for the many lines that are not main-zone state (`SVOFF`, `SYSDA PCM`, other zones and so on).
pub fn parse_line(line: &str) -> Option<Push> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("MVMAX ") {
        return parse_level(rest.trim()).map(Push::MaxVolume);
    }
    if let Some(rest) = line.strip_prefix("PW") {
        return Some(Push::Power(match rest {
            "ON" => Power::On,
            "STANDBY" | "OFF" => Power::Standby,
            other if !other.is_empty() => Power::Other(other.to_owned()),
            _ => return None,
        }));
    }
    if let Some(rest) = line.strip_prefix("MV") {
        return parse_level(rest).map(|level| Push::Volume(level - 80.0));
    }
    if let Some(rest) = line.strip_prefix("MU") {
        return match rest {
            "ON" => Some(Push::Mute(true)),
            "OFF" => Some(Push::Mute(false)),
            _ => None,
        };
    }
    if let Some(rest) = line.strip_prefix("SI") {
        return (!rest.is_empty()).then(|| Push::Input(rest.to_owned()));
    }
    if let Some(rest) = line.strip_prefix("MS") {
        // `MSQUICK1` and `MSSMART1` are quick-select slots, not a sound mode
        let mode = rest.trim();
        return (!mode.is_empty() && !mode.starts_with("QUICK") && !mode.starts_with("SMART"))
            .then(|| Push::Mode(mode.to_owned()));
    }
    if let Some(rest) = line.strip_prefix("SYSDA") {
        let text = rest.trim();
        return (!text.is_empty()).then(|| Push::Signal(text.to_owned()));
    }
    None
}

/// A volume as the receiver writes it on the panel's scale: two digits are whole steps (`35`), three are tenths (`355` is 35.5,
/// `055` is 5.5).
fn parse_level(digits: &str) -> Option<f64> {
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match digits.len() {
        2 => digits.parse::<f64>().ok(),
        3 => digits.parse::<f64>().ok().map(|v| v / 10.0),
        _ => None,
    }
}

/// What the receiver has said so far, kept until all four parts are known.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    power: Option<Power>,
    volume_db: Option<f64>,
    mute: Option<bool>,
    input: Option<String>,
    mode: Option<String>,
    signal: Option<String>,
    max_volume: Option<f64>,
}

impl Snapshot {
    /// Takes one push in.
    pub fn apply(&mut self, push: Push) {
        match push {
            Push::Power(p) => self.power = Some(p),
            Push::Volume(db) => self.volume_db = Some(db),
            Push::Mute(m) => self.mute = Some(m),
            Push::Input(i) => self.input = Some(i),
            Push::Mode(m) => self.mode = Some(m),
            Push::Signal(s) => self.signal = Some(s),
            Push::MaxVolume(v) => self.max_volume = Some(v),
        }
    }

    /// The status once power, volume, mute and input are all known. The display mode is not on this port, so it is
    /// [`VolumeDisplay::Unknown`] for the caller to fill in.
    pub fn status(&self) -> Option<Status> {
        Some(Status {
            power: self.power.clone()?,
            input: Some(self.input.clone()?),
            volume_display: VolumeDisplay::Unknown,
            volume_db: Some(self.volume_db?),
            mute: Some(self.mute?),
        })
    }

    /// The sound mode last heard, if any.
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    /// The input signal's words last heard, if any.
    pub fn signal(&self) -> Option<&str> {
        self.signal.as_deref()
    }

    /// The receiver's volume limit (0 to 98 scale) as last heard, if any.
    pub fn max_volume(&self) -> Option<f64> {
        self.max_volume
    }
}

/// One telnet connection. It never reconnects by itself: when it fails the caller drops it and decides what next.
#[derive(Debug)]
pub struct Session {
    stream: TcpStream,
    pending: Vec<u8>,
}

impl Session {
    /// Connects to the receiver's `port` within `timeout`.
    pub fn open(client: &Client, port: u16, timeout: Duration) -> Result<Self, ClientError> {
        let stream = client.connect(port, timeout)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(|e| ClientError::Io(short(&e)))?;
        Ok(Self {
            stream,
            pending: Vec::new(),
        })
    }

    /// A second handle on the same socket, so another thread can [`close`] it and wake a read that is waiting.
    pub fn closer(&self) -> std::io::Result<TcpStream> {
        self.stream.try_clone()
    }

    /// Sends one query (`MV?`: 2 to 16 characters of `A-Z`, `0-9`, ending in `?`) and its line end. Anything else is refused
    /// without writing: this connection only reads.
    pub fn query(&mut self, query: &str) -> Result<(), ClientError> {
        let valid = (2..=16).contains(&query.len())
            && query.ends_with('?')
            && query[..query.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ');
        if !valid {
            return Err(ClientError::InvalidCommand);
        }
        let mut line = Vec::with_capacity(query.len() + 1);
        line.extend_from_slice(query.as_bytes());
        line.push(b'\r');
        self.stream.write_all(&line).map_err(|e| io_error(&e))
    }

    /// Waits up to `wait` for data and appends the whole lines that arrived to `lines`. `Ok(true)` if any bytes came,
    /// `Ok(false)` if the wait ended quietly. The receiver closing the connection is an error.
    pub fn read(&mut self, wait: Duration, lines: &mut Vec<String>) -> Result<bool, ClientError> {
        self.stream
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
            .map_err(|e| ClientError::Io(short(&e)))?;
        let mut chunk = [0u8; 1024];
        let n = match self.stream.read(&mut chunk) {
            Ok(0) => return Err(ClientError::Io("closed by the receiver".to_owned())),
            Ok(n) => n,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(false);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => return Ok(false),
            Err(e) => return Err(io_error(&e)),
        };
        self.pending.extend_from_slice(&chunk[..n]);
        while let Some(end) = self.pending.iter().position(|b| matches!(b, b'\r' | b'\n')) {
            let text = String::from_utf8_lossy(&self.pending[..end])
                .trim()
                .to_owned();
            self.pending.drain(..=end);
            if !text.is_empty() {
                lines.push(text);
            }
        }
        if self.pending.len() > MAX_LINE {
            return Err(ClientError::BadResponse("line too long"));
        }
        Ok(true)
    }
}

/// Closes the socket behind `closer` (from [`Session::closer`]); a read waiting on it returns at once.
pub fn close(closer: &TcpStream) {
    let _ = closer.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn lines_read_like_the_receiver_writes_them() {
        assert_eq!(parse_line("PWON"), Some(Push::Power(Power::On)));
        assert_eq!(parse_line("PWSTANDBY"), Some(Push::Power(Power::Standby)));
        assert_eq!(
            parse_line("PWFOO"),
            Some(Push::Power(Power::Other("FOO".into())))
        );
        assert_eq!(parse_line("PW"), None);
        assert_eq!(parse_line("MV355"), Some(Push::Volume(-44.5)));
        assert_eq!(parse_line("MV36"), Some(Push::Volume(-44.0)));
        assert_eq!(parse_line("MV055"), Some(Push::Volume(-74.5)));
        assert_eq!(parse_line("MV00"), Some(Push::Volume(-80.0)));
        assert_eq!(parse_line("MV980"), Some(Push::Volume(18.0)));
        assert_eq!(parse_line("MUON"), Some(Push::Mute(true)));
        assert_eq!(parse_line("MUOFF"), Some(Push::Mute(false)));
        assert_eq!(parse_line("SITV"), Some(Push::Input("TV".into())));
        assert_eq!(parse_line("SISAT/CBL"), Some(Push::Input("SAT/CBL".into())));
        assert_eq!(parse_line(" MUON "), Some(Push::Mute(true)));
    }

    #[test]
    fn lines_that_are_not_main_zone_state_are_ignored() {
        for line in [
            "MVMAX",
            "MVMAX x",
            "MVMAX 6655",
            "SVOFF",
            "SYSMI Dolby Audio - Dolby Surround",
            "Z2ON",
            "Z2MUON",
            "MV",
            "MV3",
            "MV3555",
            "MV3x",
            "MU",
            "MUMAYBE",
            "SI",
            "SSINFAISSIG 13",
            "PSTONE CTRL OFF",
            "",
        ] {
            assert_eq!(parse_line(line), None, "{line:?}");
        }
    }

    #[test]
    fn the_volume_limit_is_on_the_panels_scale() {
        assert_eq!(parse_line("MVMAX 665"), Some(Push::MaxVolume(66.5)));
        assert_eq!(parse_line("MVMAX 81"), Some(Push::MaxVolume(81.0)));
        let mut snapshot = Snapshot::default();
        assert_eq!(snapshot.max_volume(), None);
        snapshot.apply(parse_line("MVMAX 665").unwrap());
        assert_eq!(snapshot.max_volume(), Some(66.5));
    }

    #[test]
    fn a_status_needs_all_four_parts() {
        let mut snapshot = Snapshot::default();
        for line in ["PWON", "MV355", "MUOFF"] {
            snapshot.apply(parse_line(line).unwrap());
            assert!(snapshot.status().is_none());
        }
        snapshot.apply(parse_line("SITV").unwrap());
        let status = snapshot.status().unwrap();
        assert_eq!(status.power, Power::On);
        assert_eq!(status.volume_db, Some(-44.5));
        assert_eq!(status.mute, Some(false));
        assert_eq!(status.input.as_deref(), Some("TV"));
        assert_eq!(status.volume_display, VolumeDisplay::Unknown);
        snapshot.apply(parse_line("MV36").unwrap());
        assert_eq!(snapshot.status().unwrap().volume_db, Some(-44.0));
    }

    fn pair() -> (Session, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = Client::new("127.0.0.1", 1);
        let session = Session::open(&client, port, Duration::from_secs(2)).unwrap();
        let (peer, _) = listener.accept().unwrap();
        (session, peer)
    }

    #[test]
    fn only_queries_are_sent_and_they_end_with_a_carriage_return() {
        let (mut session, mut peer) = pair();
        for bad in [
            "MV35",
            "MUON",
            "PW?\r",
            "pw?",
            "?",
            "MV?;MUON",
            "ABCDEFGHIJKLMNOPQ?",
        ] {
            assert_eq!(
                session.query(bad),
                Err(ClientError::InvalidCommand),
                "{bad}"
            );
        }
        session.query("MV?").unwrap();
        session.query("PW?").unwrap();
        let mut got = [0u8; 8];
        peer.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"MV?\rPW?\r");
    }

    #[test]
    fn lines_are_cut_at_either_line_end_and_a_split_line_waits_for_its_rest() {
        let (mut session, mut peer) = pair();
        let mut lines = Vec::new();
        peer.write_all(b"MV355\rMVMAX 665\r\nMU").unwrap();
        assert!(session.read(Duration::from_secs(2), &mut lines).unwrap());
        assert_eq!(lines, ["MV355", "MVMAX 665"]);
        lines.clear();
        peer.write_all(b"OFF\r").unwrap();
        assert!(session.read(Duration::from_secs(2), &mut lines).unwrap());
        assert_eq!(lines, ["MUOFF"]);
        // A quiet wait is not an error.
        lines.clear();
        assert!(!session.read(Duration::from_millis(20), &mut lines).unwrap());
        assert!(lines.is_empty());
    }

    #[test]
    fn a_closed_connection_and_an_endless_line_are_errors() {
        let (mut session, peer) = pair();
        drop(peer);
        let mut lines = Vec::new();
        assert!(matches!(
            session.read(Duration::from_secs(2), &mut lines),
            Err(ClientError::Io(_))
        ));

        let (mut session, mut peer) = pair();
        peer.write_all(&vec![b'x'; MAX_LINE + 100]).unwrap();
        let mut result = Ok(false);
        for _ in 0..8 {
            result = session.read(Duration::from_secs(2), &mut lines);
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(ClientError::BadResponse("line too long")));
    }

    #[test]
    fn closing_the_other_handle_wakes_a_waiting_read() {
        let (mut session, _peer) = pair();
        let closer = session.closer().unwrap();
        let waiter = std::thread::spawn(move || {
            let mut lines = Vec::new();
            session.read(Duration::from_secs(30), &mut lines)
        });
        std::thread::sleep(Duration::from_millis(50));
        close(&closer);
        assert!(waiter.join().unwrap().is_err());
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn the_sound_mode_and_the_signal_words_are_heard_as_the_receiver_says_them() {
        assert_eq!(
            parse_line("MSDOLBY AUDIO-DSUR"),
            Some(Push::Mode("DOLBY AUDIO-DSUR".into()))
        );
        assert_eq!(parse_line("MSSTEREO"), Some(Push::Mode("STEREO".into())));
        assert_eq!(
            parse_line("SYSDA PCM ZERO             "),
            Some(Push::Signal("PCM ZERO".into()))
        );
        assert_eq!(parse_line("SYSMI Dolby Audio - Dolby Surround"), None);
        assert_eq!(parse_line("SSINFAISSIG 13"), None);
    }

    #[test]
    fn quick_select_slots_and_empty_lines_are_not_a_sound_mode() {
        assert_eq!(parse_line("MSQUICK1"), None);
        assert_eq!(parse_line("MSSMART2"), None);
        assert_eq!(parse_line("MS"), None);
        assert_eq!(parse_line("SYSDA"), None);
    }

    #[test]
    fn the_snapshot_keeps_them_beside_the_four_parts_and_is_ready_without_them() {
        let mut s = Snapshot::default();
        for push in [
            Push::Power(Power::On),
            Push::Volume(-37.5),
            Push::Mute(false),
            Push::Input("TV".into()),
        ] {
            s.apply(push);
        }
        assert!(s.status().is_some() && s.mode().is_none() && s.signal().is_none());
        s.apply(Push::Mode("STEREO".into()));
        s.apply(Push::Signal("PCM ZERO".into()));
        assert_eq!((s.mode(), s.signal()), (Some("STEREO"), Some("PCM ZERO")));
    }

    #[test]
    fn the_format_queries_are_queries_a_space_is_allowed_inside_and_a_command_still_is_not() {
        for q in FORMAT_QUERIES {
            assert!(q.ends_with('?') && (2..=16).contains(&q.len()));
        }
        assert!(
            "SSINFAISSIG ?"
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ' || b == b'?')
        );
    }
}
