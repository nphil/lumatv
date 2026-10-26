//! A small MQTT 3.1.1 client for the Zigbee2MQTT output: what the helper needs of a broker on the home network and nothing more.
//!
//! CONNECT with a clean session, user name and password; PUBLISH at QoS 0 (a frame that is late is worse than one that is lost,
//! and the broker and Zigbee2MQTT are on the same LAN); SUBSCRIBE at QoS 0 with the SUBACK waited for; PINGREQ when idle;
//! DISCONNECT. Incoming packets are parsed from a buffer that is filled by whatever a read returns, so a retained message of any
//! size (Zigbee2MQTT's device list runs to hundreds of KB) arrives whole however it is split, and a timeout in the middle of one
//! loses nothing. Blocking `std` sockets with timeouts: the output thread is the only caller and does nothing else meanwhile.
//!
//! The password goes on the wire once, in CONNECT, and nowhere else: no error carries it and nothing here logs.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// A packet larger than this is not something the helper expects from a broker on a home network.
const MAX_PACKET: usize = 16 * 1024 * 1024;
/// How much is read at a time.
const CHUNK: usize = 64 * 1024;

/// What the broker is told at CONNECT.
pub struct Options<'a> {
    pub client_id: &'a str,
    pub username: Option<&'a str>,
    pub password: Option<&'a str>,
    pub keep_alive: Duration,
    /// For the TCP connect and for each wait on an answer (CONNACK, SUBACK).
    pub timeout: Duration,
}

/// A message the broker delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The broker refused the connection: its reason in words.
    Refused(&'static str),
    /// Something that is not MQTT 3.1.1, or not an answer to what was asked.
    Protocol(String),
    Timeout(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Refused(why) => write!(f, "the broker refused the connection: {why}"),
            Error::Protocol(why) => {
                write!(f, "not an MQTT broker, or not one this understands: {why}")
            }
            Error::Timeout(what) => write!(f, "the broker did not answer the {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// The words of a CONNACK return code (MQTT 3.1.1, 3.2.2.3).
fn connack_words(code: u8) -> &'static str {
    match code {
        1 => "it does not speak MQTT 3.1.1",
        2 => "it rejected the client id",
        3 => "the service is unavailable",
        4 => "bad user name or password",
        5 => "not authorised",
        _ => "an unknown reason",
    }
}

pub struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    keep_alive: Duration,
    timeout: Duration,
    last_sent: Instant,
    ping_sent: Option<Instant>,
    next_id: u16,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let len = u16::try_from(s.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&s.as_bytes()[..usize::from(len)]);
}

fn put_remaining_length(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut byte = u8::try_from(n % 128).unwrap_or(0);
        n /= 128;
        if n > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
}

fn packet(kind_and_flags: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(kind_and_flags);
    put_remaining_length(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

/// The CONNECT packet's bytes.
fn connect_packet(o: &Options<'_>) -> Vec<u8> {
    let mut body = Vec::new();
    put_str(&mut body, "MQTT");
    body.push(4);
    let mut flags = 0x02; // clean session
    if o.username.is_some() {
        flags |= 0x80;
    }
    if o.password.is_some() {
        flags |= 0x40;
    }
    body.push(flags);
    let secs = u16::try_from(o.keep_alive.as_secs()).unwrap_or(u16::MAX);
    body.extend_from_slice(&secs.to_be_bytes());
    put_str(&mut body, o.client_id);
    if let Some(u) = o.username {
        put_str(&mut body, u);
    }
    if let Some(p) = o.password {
        put_str(&mut body, p);
    }
    packet(0x10, &body)
}

/// A complete packet at the front of `buf`: (first byte, body length, total length), or `None` when more is needed.
fn framed(buf: &[u8]) -> Result<Option<(u8, usize, usize)>, Error> {
    let Some(&first) = buf.first() else {
        return Ok(None);
    };
    let mut len: usize = 0;
    let mut shift = 0;
    for (i, &b) in buf.iter().enumerate().skip(1) {
        if i > 4 {
            return Err(Error::Protocol("a malformed length".to_owned()));
        }
        len |= usize::from(b & 0x7F) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if len > MAX_PACKET {
                return Err(Error::Protocol(format!("a packet of {len} bytes")));
            }
            let total = i + 1 + len;
            return Ok((buf.len() >= total).then_some((first, len, total)));
        }
    }
    Ok(None)
}

impl Client {
    /// Connects and waits for the broker's acceptance.
    pub fn connect(addr: SocketAddr, o: &Options<'_>) -> Result<Client, Error> {
        let stream = TcpStream::connect_timeout(&addr, o.timeout)?;
        stream.set_nodelay(true)?;
        stream.set_write_timeout(Some(o.timeout))?;
        let mut client = Client {
            stream,
            buf: Vec::new(),
            keep_alive: o.keep_alive,
            timeout: o.timeout,
            last_sent: Instant::now(),
            ping_sent: None,
            next_id: 1,
        };
        client.send(&connect_packet(o))?;
        let (first, body) = client.expect(0x20, "connection", true)?;
        if first & 0x0F != 0 || body.len() != 2 {
            return Err(Error::Protocol("a malformed CONNACK".to_owned()));
        }
        match body[1] {
            0 => Ok(client),
            code => Err(Error::Refused(connack_words(code))),
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.stream.write_all(bytes)?;
        self.last_sent = Instant::now();
        Ok(())
    }

    /// Publishes at QoS 0, not retained.
    pub fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), Error> {
        let mut body = Vec::with_capacity(2 + topic.len() + payload.len());
        put_str(&mut body, topic);
        body.extend_from_slice(payload);
        self.send(&packet(0x30, &body))
    }

    /// Subscribes at QoS 0 and waits for the SUBACK. Messages that arrive meanwhile are kept for [`Client::poll`].
    pub fn subscribe(&mut self, filter: &str) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let mut body = Vec::new();
        body.extend_from_slice(&id.to_be_bytes());
        put_str(&mut body, filter);
        body.push(0);
        self.send(&packet(0x82, &body))?;
        let (_, ack) = self.expect(0x90, "subscription", false)?;
        if ack.len() < 3 || ack[..2] != id.to_be_bytes() {
            return Err(Error::Protocol(
                "a SUBACK for another subscription".to_owned(),
            ));
        }
        if ack[2] == 0x80 {
            return Err(Error::Refused("the subscription was not allowed"));
        }
        Ok(())
    }

    /// Reads until a packet of `kind` arrives (within the timeout); other packets are handled or kept.
    fn expect(
        &mut self,
        kind: u8,
        what: &'static str,
        first: bool,
    ) -> Result<(u8, Vec<u8>), Error> {
        let deadline = Instant::now() + self.timeout;
        let mut kept = Vec::new();
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.buf.splice(0..0, kept);
                return Err(Error::Timeout(what));
            }
            match self.next_packet(deadline - now)? {
                Some((f, body)) if f & 0xF0 == kind => {
                    self.buf.splice(0..0, kept);
                    return Ok((f, body));
                }
                Some((f, body)) if first => {
                    // Before CONNACK nothing else is legitimate.
                    let _ = body;
                    return Err(Error::Protocol(format!("packet 0x{f:02x} before CONNACK")));
                }
                Some((f, body)) => {
                    // A message that came early: put it back for `poll`, whole.
                    let mut raw = packet(f, &body);
                    kept.append(&mut raw);
                }
                None => {}
            }
        }
    }

    /// The next complete packet, reading for up to `wait`. A zero wait still looks at the socket once (for a millisecond), so
    /// what the broker has pushed is taken between frames.
    fn next_packet(&mut self, wait: Duration) -> Result<Option<(u8, Vec<u8>)>, Error> {
        let deadline = Instant::now() + wait;
        let mut looked = false;
        loop {
            if let Some((first, len, total)) = framed(&self.buf)? {
                let body = self.buf[total - len..total].to_vec();
                self.buf.drain(..total);
                return Ok(Some((first, body)));
            }
            let now = Instant::now();
            if now >= deadline && looked {
                return Ok(None);
            }
            looked = true;
            self.stream.set_read_timeout(Some(
                deadline
                    .saturating_duration_since(now)
                    .max(Duration::from_millis(1)),
            ))?;
            let start = self.buf.len();
            self.buf.resize(start + CHUNK, 0);
            match self.stream.read(&mut self.buf[start..]) {
                Ok(0) => {
                    self.buf.truncate(start);
                    return Err(Error::Io(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "the broker closed the connection",
                    )));
                }
                Ok(n) => self.buf.truncate(start + n),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    self.buf.truncate(start);
                    return Ok(None);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => self.buf.truncate(start),
                Err(e) => {
                    self.buf.truncate(start);
                    return Err(Error::Io(e));
                }
            }
        }
    }

    /// Sends a PINGREQ when the connection has been quiet for half the keep-alive, and gives up on a broker that does not answer
    /// one within the keep-alive.
    fn keep_alive(&mut self) -> Result<(), Error> {
        let now = Instant::now();
        if let Some(sent) = self.ping_sent {
            if now.saturating_duration_since(sent) > self.keep_alive {
                return Err(Error::Timeout("ping"));
            }
        } else if now.saturating_duration_since(self.last_sent) >= self.keep_alive / 2 {
            self.send(&[0xC0, 0x00])?;
            self.ping_sent = Some(now);
        }
        Ok(())
    }

    /// The next message, waiting up to `wait` for one; `None` when nothing came. Keeps the connection alive meanwhile.
    pub fn poll(&mut self, wait: Duration) -> Result<Option<Message>, Error> {
        let deadline = Instant::now() + wait;
        loop {
            self.keep_alive()?;
            let now = Instant::now();
            let slice = deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(250));
            let Some((first, body)) = self.next_packet(slice)? else {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                continue;
            };
            match first & 0xF0 {
                0x30 => {
                    let qos = (first >> 1) & 0x03;
                    if body.len() < 2 {
                        return Err(Error::Protocol("a malformed PUBLISH".to_owned()));
                    }
                    let tlen = usize::from(u16::from_be_bytes([body[0], body[1]]));
                    let id_len = if qos > 0 { 2 } else { 0 };
                    if body.len() < 2 + tlen + id_len {
                        return Err(Error::Protocol("a malformed PUBLISH".to_owned()));
                    }
                    let topic = String::from_utf8_lossy(&body[2..2 + tlen]).into_owned();
                    if qos == 1 {
                        let id = &body[2 + tlen..4 + tlen];
                        self.send(&[0x40, 0x02, id[0], id[1]])?;
                    }
                    let payload = body[2 + tlen + id_len..].to_vec();
                    return Ok(Some(Message {
                        topic,
                        payload,
                        retain: first & 0x01 != 0,
                    }));
                }
                0xD0 => self.ping_sent = None,
                _ => {}
            }
        }
    }

    /// Tells the broker goodbye and closes the socket. Nothing is waited for.
    pub fn disconnect(mut self) {
        let _ = self
            .stream
            .set_write_timeout(Some(Duration::from_millis(500)));
        let _ = self.stream.write_all(&[0xE0, 0x00]);
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_lengths_are_the_specifications_examples() {
        let enc = |n: usize| {
            let mut v = Vec::new();
            put_remaining_length(&mut v, n);
            v
        };
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(127), [0x7F]);
        assert_eq!(enc(128), [0x80, 0x01]);
        assert_eq!(enc(16_383), [0xFF, 0x7F]);
        assert_eq!(enc(16_384), [0x80, 0x80, 0x01]);
        assert_eq!(enc(2_097_151), [0xFF, 0xFF, 0x7F]);
        assert_eq!(enc(2_097_152), [0x80, 0x80, 0x80, 0x01]);
    }

    #[test]
    fn a_packet_is_framed_only_when_whole() {
        let p = packet(0x30, &[1u8; 300]);
        assert_eq!(framed(&p[..1]).unwrap(), None);
        assert_eq!(framed(&p[..2]).unwrap(), None);
        assert_eq!(framed(&p[..100]).unwrap(), None);
        assert_eq!(framed(&p).unwrap(), Some((0x30, 300, 303)));
        let mut two = p.clone();
        two.extend_from_slice(&[0xD0, 0x00]);
        assert_eq!(framed(&two).unwrap(), Some((0x30, 300, 303)));
        assert!(framed(&[0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]).is_err());
    }

    #[test]
    fn connect_carries_the_flags_and_fields_in_order() {
        let o = Options {
            client_id: "tv",
            username: Some("u"),
            password: Some("p"),
            keep_alive: Duration::from_secs(30),
            timeout: Duration::from_secs(1),
        };
        let bytes = connect_packet(&o);
        let body = &bytes[2..];
        assert_eq!(bytes[0], 0x10);
        assert_eq!(&body[..7], &[0, 4, b'M', b'Q', b'T', b'T', 4]);
        assert_eq!(body[7], 0x02 | 0x80 | 0x40);
        assert_eq!(&body[8..10], &[0, 30]);
        assert_eq!(&body[10..], &[0, 2, b't', b'v', 0, 1, b'u', 0, 1, b'p']);
        let anon = connect_packet(&Options {
            username: None,
            password: None,
            ..o
        });
        assert_eq!(anon[2 + 7], 0x02);
        assert_eq!(&anon[2 + 10..], &[0, 2, b't', b'v']);
    }
}
