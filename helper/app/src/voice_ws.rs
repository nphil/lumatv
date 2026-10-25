//! A blocking WebSocket conversation with Home Assistant, for one voice request.
//!
//! The protocol is LumaShow's `ha-link` crate's (`ws` frames, the TCP and `openssl s_client` transports, the address rules: the token
//! is dialled only to the home network). What it adds is what a voice request needs and the link's subscription thread does not:
//! binary frames out (the audio) and a read with a deadline, on one thread that owns the connection for the length of one request.
//!
//! Nothing here logs or returns the token; a failure is a [`Fail`] in words.

use crate::voice_store::HaLink;
use fetch::TlsFetch;
use ha_link::endpoint::AddressPolicy;
use ha_link::transport::{Tcp, TlsChild, Transport};
use ha_link::ws::{self, Decoder, Masker, Message};
use serde_json::{Value, json};
use std::io;
use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

/// Why the conversation could not go on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
    /// No connection was made, or the opening handshake failed (in words).
    Cannot(String),
    /// Home Assistant refused the token.
    AuthInvalid,
    /// The connection ended or Home Assistant sent something that cannot be read (in words).
    Lost(String),
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cannot(why) | Self::Lost(why) => f.write_str(why),
            Self::AuthInvalid => f.write_str("Home Assistant refused the access token"),
        }
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(160)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// An open, authenticated connection.
pub struct Ha {
    transport: Box<dyn Transport>,
    decoder: Decoder,
    masker: Masker,
    mask: u64,
}

impl Ha {
    /// Dials Home Assistant, upgrades to a WebSocket and authenticates, all before `deadline`. `tls` is what a secure address is
    /// dialled with; without it a secure address is refused.
    pub fn connect(
        link: &HaLink,
        policy: AddressPolicy,
        tls: Option<&TlsFetch>,
        deadline: Instant,
    ) -> Result<Self, Fail> {
        let endpoint = &link.endpoint;
        let addrs = endpoint
            .resolve(policy)
            .map_err(|e| Fail::Cannot(clean(&e.to_string())))?;
        let transport: Box<dyn Transport> =
            if endpoint.secure {
                let Some(tls) = tls else {
                    return Err(Fail::Cannot(
                        "a secure address needs the system's TLS helper, which is not here".into(),
                    ));
                };
                // The address that was checked is the one dialled.
                let connect = match addrs[0] {
                    SocketAddr::V4(a) => a.to_string(),
                    SocketAddr::V6(a) => format!("[{}]:{}", a.ip(), a.port()),
                };
                let command = tls.stream_command(&connect, &endpoint.host, endpoint.ip);
                Box::new(TlsChild::spawn(command).map_err(|e| {
                    Fail::Cannot(clean(&format!("cannot start the TLS helper: {e}")))
                })?)
            } else {
                let left = deadline.saturating_duration_since(Instant::now());
                Box::new(Tcp::connect(&addrs, left).map_err(|e| {
                    Fail::Cannot(clean(&format!(
                        "cannot connect to {}: {e}",
                        endpoint.authority()
                    )))
                })?)
            };
        let mut ha = Self {
            transport,
            decoder: Decoder::new(),
            masker: Masker::from_system(),
            mask: ws::random_u64() | 1,
        };
        ha.open(link, deadline)?;
        Ok(ha)
    }

    /// The upgrade, then `auth_required`, the token, `auth_ok`.
    fn open(&mut self, link: &HaLink, deadline: Instant) -> Result<(), Fail> {
        let key = ws::new_key();
        let request = ws::request(&link.endpoint.authority(), &link.endpoint.path(), &key);
        self.write_all(&request, deadline)?;
        let mut head = Vec::new();
        let used = loop {
            match ws::read_response(&head, &key) {
                Ok(Some(used)) => break used,
                Ok(None) => {}
                Err(e) => return Err(Fail::Cannot(clean(&e.to_string()))),
            }
            if !self.read_into(&mut head, deadline)? {
                return Err(Fail::Cannot("Home Assistant did not answer in time".into()));
            }
        };
        self.decoder.push(&head[used..]);
        let mut sent = false;
        loop {
            let Some(message) = self.recv(deadline.saturating_duration_since(Instant::now()))?
            else {
                return Err(Fail::Cannot("Home Assistant did not answer in time".into()));
            };
            match (message["type"].as_str().unwrap_or(""), sent) {
                ("auth_required", false) => {
                    let auth = json!({"type": "auth", "access_token": link.token.expose()});
                    self.send_json(&auth, deadline)?;
                    sent = true;
                }
                ("auth_ok", true) => return Ok(()),
                ("auth_invalid", true) => return Err(Fail::AuthInvalid),
                _ => return Err(Fail::Cannot("Home Assistant spoke out of turn".into())),
            }
        }
    }

    /// One JSON text message.
    pub fn send_json(&mut self, value: &Value, deadline: Instant) -> Result<(), Fail> {
        let frame = ws::text(&mut self.masker, value.to_string().as_bytes());
        self.write_all(&frame, deadline)
    }

    /// One binary message: the handler's id (the one `run-start` named) and then the audio. Just the id is the end of the audio.
    pub fn send_audio(&mut self, handler: u8, pcm: &[u8], deadline: Instant) -> Result<(), Fail> {
        let mut payload = Vec::with_capacity(pcm.len() + 1);
        payload.push(handler);
        payload.extend_from_slice(pcm);
        let frame = self.binary_frame(&payload);
        self.write_all(&frame, deadline)
    }

    /// A masked binary frame (`ha-link`'s own frame builder has no binary opcode).
    fn binary_frame(&mut self, payload: &[u8]) -> Vec<u8> {
        // xorshift: a client's mask has to be unpredictable to what is in between, not secret from the peer.
        let mut x = self.mask;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.mask = x;
        let mask = ((x >> 16) as u32).to_le_bytes();
        let mut out = Vec::with_capacity(payload.len() + 14);
        out.push(0x80 | 0x2);
        let len = payload.len();
        if len < 126 {
            out.push(0x80 | len as u8);
        } else if len <= usize::from(u16::MAX) {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    /// Says goodbye (best effort) and drops the connection.
    pub fn close(mut self) {
        let frame = ws::close(&mut self.masker, 1000);
        let _ = self.write_all(&frame, Instant::now() + Duration::from_millis(300));
    }

    /// The next JSON message within `wait`; `None` when none came. Pings are answered; a close or an unreadable message is a
    /// failure.
    pub fn recv(&mut self, wait: Duration) -> Result<Option<Value>, Fail> {
        let deadline = Instant::now() + wait;
        loop {
            match self.decoder.next_message() {
                Ok(Some(Message::Text(text))) => {
                    return serde_json::from_str::<Value>(&text).map(Some).map_err(|_| {
                        Fail::Lost("Home Assistant sent something unreadable".into())
                    });
                }
                Ok(Some(Message::Ping(payload))) => {
                    let frame = ws::pong(&mut self.masker, &payload);
                    self.write_all(&frame, Instant::now() + Duration::from_secs(2))?;
                    continue;
                }
                Ok(Some(Message::Close(_))) => {
                    return Err(self.lost("Home Assistant closed the connection"));
                }
                Ok(Some(Message::Binary(_) | Message::Pong)) => continue,
                Err(e) => return Err(Fail::Lost(clean(&e.to_string()))),
                Ok(None) => {}
            }
            let mut got = Vec::new();
            if !self.read_into(&mut got, deadline)? {
                return Ok(None);
            }
            self.decoder.push(&got);
        }
    }

    fn lost(&mut self, error: &str) -> Fail {
        match self.transport.why_closed() {
            Some(why) => Fail::Lost(why),
            None => Fail::Lost(clean(error)),
        }
    }

    /// Waits for bytes until `deadline` and appends them to `into`. False at the deadline.
    fn read_into(&mut self, into: &mut Vec<u8>, deadline: Instant) -> Result<bool, Fail> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if !poll_one(self.transport.read_fd(), libc::POLLIN, left) {
                return Ok(false);
            }
            let mut buf = [0u8; 8 * 1024];
            match self.transport.read(&mut buf) {
                Ok(0) => return Err(self.lost("Home Assistant closed the connection")),
                Ok(n) => {
                    into.extend_from_slice(&buf[..n]);
                    return Ok(true);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(self.lost(&e.to_string())),
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
        }
    }

    fn write_all(&mut self, data: &[u8], deadline: Instant) -> Result<(), Fail> {
        let mut sent = 0;
        while sent < data.len() {
            match self.transport.write(&data[sent..]) {
                Ok(0) => return Err(self.lost("the connection to Home Assistant closed")),
                Ok(n) => sent += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(Fail::Lost("Home Assistant does not read".into()));
                    }
                    poll_one(self.transport.write_fd(), libc::POLLOUT, left);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(self.lost(&e.to_string())),
            }
        }
        Ok(())
    }
}

/// Waits up to `timeout` for `events` on `fd`. True when it is ready (an error or a hang-up counts: the read that follows says
/// what it was).
fn poll_one(fd: RawFd, events: libc::c_short, timeout: Duration) -> bool {
    let mut polled = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // Rounded up: waking a hair early would report a time that has not quite come.
    let millis =
        i32::try_from(timeout.as_millis() + u128::from(timeout.subsec_nanos() % 1_000_000 != 0))
            .unwrap_or(i32::MAX);
    // SAFETY: one live pollfd structure and its length.
    let n = unsafe { libc::poll(&mut polled, 1, millis) };
    n > 0 && polled.revents != 0
}
