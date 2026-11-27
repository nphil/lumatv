//! Explicit approval for an RF-changing tune. Tickets live under the same lock as the leases and are cleared when those leases change.

use crate::stream::{Claim, Failure};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fmt;
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

pub(crate) const VERSION: u8 = 1;
pub(crate) const TICKET_LIFE: Duration = Duration::from_secs(30);
const MAX_TICKETS: usize = 32;

/// A privately issued bearer key, never a public stream ID or a device label.
#[derive(Clone, Copy)]
pub(crate) struct Key([u8; 16]);

/// The wire form of a key, kept on the stack and exposed only to the HTTP writer.
pub(crate) struct KeyText([u8; 32]);

impl KeyText {
    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("hexadecimal key")
    }
}

impl Key {
    pub(crate) fn new() -> Result<Self, Failure> {
        let mut random = [0; 16];
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| Failure::Hardware(format!("cannot issue tuner credential: {e}")))?;
        Ok(Self(random))
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.len() != 32 {
            return None;
        }
        let nibble = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let mut key = [0; 16];
        for (out, hex) in key.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            *out = (nibble(hex[0])? << 4) | nibble(hex[1])?;
        }
        Some(Self(key))
    }

    pub(crate) fn text(&self) -> KeyText {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut key = [0; 32];
        for (out, byte) in key.chunks_exact_mut(2).zip(&self.0) {
            out[0] = HEX[usize::from(byte >> 4)];
            out[1] = HEX[usize::from(byte & 15)];
        }
        KeyText(key)
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.0.iter().zip(other.0).fold(0, |diff, (a, b)| diff | (a ^ b)) == 0
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[private]")
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TuneRequest<'a> {
    pub(crate) peer: IpAddr,
    pub(crate) agent: Option<&'a str>,
    pub(crate) device: Option<&'a str>,
    pub(crate) profile: Option<&'a str>,
    pub(crate) structured: bool,
    pub(crate) takeover: Option<Key>,
    pub(crate) stream_key: Option<Key>,
}

impl TuneRequest<'_> {
    pub(crate) fn new(peer: IpAddr) -> Self {
        Self { peer, agent: None, device: None, profile: None, structured: false, takeover: None, stream_key: None }
    }
}

struct Ticket {
    key: Key,
    until: Duration,
    peer: IpAddr,
    device: Option<Box<str>>,
    profile: Option<Box<str>>,
    stream_key: Option<Key>,
    guide: Box<str>,
    channel_id: Box<str>,
}

impl Ticket {
    fn matches(&self, request: TuneRequest<'_>, target: &Claim, now: Duration) -> bool {
        now < self.until
            && self.peer == request.peer
            && self.device.as_deref() == request.device
            && self.profile.as_deref() == request.profile
            && self.stream_key == request.stream_key
            && *self.guide == target.guide
            && *self.channel_id == target.channel_id
    }
}

#[derive(Default)]
pub(crate) struct Tickets(VecDeque<Ticket>);

impl Tickets {
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn consume(&mut self, request: TuneRequest<'_>, target: &Claim, now: Duration) -> bool {
        let Some(key) = request.takeover else { return false };
        self.0.iter().position(|t| t.key == key && t.matches(request, target, now))
            .and_then(|i| self.0.remove(i)).is_some()
    }

    pub(crate) fn offer(&mut self, request: TuneRequest<'_>, target: &Claim, now: Duration) -> Result<Key, Failure> {
        self.0.retain(|t| now < t.until);
        if let Some(t) = self.0.iter().find(|t| t.matches(request, target, now)) {
            return Ok(t.key);
        }
        let key = Key::new()?;
        if self.0.len() == MAX_TICKETS {
            self.0.pop_front();
        }
        self.0.push_back(Ticket {
            key, until: now + TICKET_LIFE, peer: request.peer, device: request.device.map(Into::into),
            profile: request.profile.map(Into::into), stream_key: request.stream_key,
            guide: target.guide.as_str().into(), channel_id: target.channel_id.as_str().into(),
        });
        Ok(key)
    }
}

#[derive(Debug)]
pub(crate) struct Conflict {
    pub(crate) holders: Vec<Value>,
    pub(crate) takeover: Option<Key>,
    pub(crate) failure: Failure,
}

impl Conflict {
    pub(crate) fn doc(&self) -> Value {
        let takeover = self.takeover.as_ref().map(Key::text);
        json!({
            "version": VERSION,
            "error": if self.takeover.is_some() { "takeover_required" } else { "tuner_busy" },
            "takeover": takeover.as_ref().map(KeyText::as_str),
            "holders": self.holders,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offer_is_consumed_only_once_even_without_a_lease_change() {
        let peer = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let target = Claim {
            peer, display: "Player".into(), kind: "app", guide: "7.1".into(), name: "Local".into(),
            channel_id: "1_25_7_1_0_0_0".into(), since_ms: 0,
        };
        let mut tickets = Tickets::default();
        let req = TuneRequest { structured: true, ..TuneRequest::new(peer) };
        let key = tickets.offer(req, &target, Duration::ZERO).unwrap();
        let confirm = TuneRequest { takeover: Some(key), ..req };
        assert!(tickets.consume(confirm, &target, Duration::ZERO));
        assert!(!tickets.consume(confirm, &target, Duration::ZERO));
    }
}
