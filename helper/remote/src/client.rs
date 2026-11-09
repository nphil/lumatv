//! The phone's side, for the `lumaremote` test client and the tests: pairing, a session's frames, and pointer and ping datagrams.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use esphome_api::base64;
use esphome_api::noise::{CipherState, Initiator, Transport};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};

use crate::datagram::{self, Header, Key, Pointer};
use crate::link::PairLink;
use crate::pairing;
use crate::wire::{self, HANDSHAKE_LEN, Mode};

/// The longest the client waits for the TV at any step.
pub const WAIT: Duration = Duration::from_secs(5);

/// What pairing gives the phone.
#[derive(Clone, Debug)]
pub struct Paired {
    pub device: String,
    pub key: [u8; 32],
    /// `paired`'s `tv`: `id`, `name`, `mac`.
    pub tv: Value,
}

fn invalid(what: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

fn secret() -> io::Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| io::Error::other("no random source"))?;
    Ok(bytes)
}

/// The preamble and the handshake. A TV that closes instead of answering (a wrong key, no pairing window) is `UnexpectedEof`.
fn connect(addr: SocketAddr, mode: Mode, psk: &[u8; 32]) -> io::Result<(TcpStream, Transport)> {
    let mut stream = TcpStream::connect_timeout(&addr, WAIT)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    let preamble = mode.preamble();
    let (initiator, first) = Initiator::start_with(&preamble, psk, secret()?);
    stream.write_all(&[&preamble[..], &first[..]].concat())?;
    let mut second = [0u8; HANDSHAKE_LEN];
    stream.read_exact(&mut second)?;
    Ok((stream, initiator.finish(&second).map_err(invalid)?))
}

/// Pairs with the key [`pairing::key`] derives from the TV's code.
pub fn pair(addr: SocketAddr, psk: &[u8; 32], name: &str, model: &str, app: &str) -> io::Result<Paired> {
    let (mut stream, Transport { mut send, mut receive }) = connect(addr, Mode::Pair, psk)?;
    wire::write(&mut stream, &mut send, json!({"t": "pair", "name": name, "model": model, "app": app}).to_string().as_bytes())?;
    let reply: Value = serde_json::from_slice(&wire::Reader::default().next(&mut stream, &mut receive)?)?;
    if reply["t"] != "paired" {
        return Err(io::Error::other(format!("the TV refused: {reply}")));
    }
    let device = reply["device"].as_str().ok_or_else(|| invalid("paired without a device"))?.to_string();
    let key = reply["key"].as_str().and_then(base64::decode_key).ok_or_else(|| invalid("paired without a key"))?;
    Ok(Paired { device, key, tv: reply["tv"].clone() })
}

/// Pairs by a scanned link, whose code and TV id give the key.
pub fn pair_link(link: &PairLink, name: &str, model: &str, app: &str) -> io::Result<Paired> {
    let addr = resolve(&link.host, link.port)?;
    pair(addr, &pairing::key(&link.code, &link.tv), name, model, app)
}

/// The first IPv4 address of `host`.
pub fn resolve(host: &str, port: u16) -> io::Result<SocketAddr> {
    (host, port).to_socket_addrs()?.find(SocketAddr::is_ipv4).ok_or_else(|| io::Error::other(format!("{host}: no IPv4 address")))
}

/// A live session: frames over TCP, pointer and ping datagrams over UDP. Dropping it says `bye`.
pub struct Session {
    /// The TV's `welcome`.
    pub welcome: Value,
    stream: TcpStream,
    send: CipherState,
    receive: CipherState,
    reader: wire::Reader,
    udp: UdpSocket,
    udp_key: Key,
    udp_session: u32,
    seq: u32,
    /// The last datagram sent, for [`Session::resend`].
    last: Vec<u8>,
}

impl Session {
    /// The handshake with the phone's key, then `hello`; returns once `welcome` is in.
    pub fn open(addr: SocketAddr, key: &[u8; 32], hello: &Value) -> io::Result<Session> {
        let (mut stream, Transport { mut send, mut receive }) = connect(addr, Mode::Session, key)?;
        wire::write(&mut stream, &mut send, hello.to_string().as_bytes())?;
        let mut reader = wire::Reader::default();
        let welcome: Value = serde_json::from_slice(&reader.next(&mut stream, &mut receive)?)?;
        if welcome["t"] != "welcome" {
            return Err(io::Error::other(format!("the TV refused: {welcome}")));
        }
        let udp_session = welcome["udp"]["session"].as_u64().and_then(|s| u32::try_from(s).ok()).ok_or_else(|| invalid("welcome without udp.session"))?;
        let udp_key = welcome["udp"]["key"].as_str().and_then(base64::decode_key).ok_or_else(|| invalid("welcome without udp.key"))?;
        let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        udp.connect(addr)?;
        udp.set_read_timeout(Some(WAIT))?;
        Ok(Session { welcome, stream, send, receive, reader, udp, udp_key: Key::new(&udp_key), udp_session, seq: 0, last: Vec::new() })
    }

    pub fn send(&mut self, msg: &Value) -> io::Result<()> {
        wire::write(&mut self.stream, &mut self.send, msg.to_string().as_bytes())
    }

    /// The TV's next frame, waiting at most [`WAIT`].
    pub fn receive(&mut self) -> io::Result<Value> {
        Ok(serde_json::from_slice(&self.reader.next(&mut self.stream, &mut self.receive)?)?)
    }

    fn datagram(&mut self, kind: u8, plain: &[u8]) -> io::Result<()> {
        self.seq += 1;
        self.last = self.udp_key.seal(datagram::TO_TV, Header { kind, session: self.udp_session, seq: self.seq }, plain);
        self.udp.send(&self.last).map(drop)
    }

    /// A pointer datagram with these totals.
    pub fn pointer(&mut self, totals: &Pointer) -> io::Result<()> {
        self.datagram(datagram::POINTER, &totals.encode())
    }

    /// The last datagram again: a replay, which the TV drops.
    pub fn resend(&self) -> io::Result<()> {
        self.udp.send(&self.last).map(drop)
    }

    /// Any bytes as a datagram.
    pub fn send_raw(&self, packet: &[u8]) -> io::Result<()> {
        self.udp.send(packet).map(drop)
    }

    /// A ping datagram; returns its pong's `id`, the time echoed and how long the TV held it in µs.
    pub fn ping(&mut self, id: u32, time: u64) -> io::Result<(u32, u64, u32)> {
        self.datagram(datagram::PING, &datagram::ping(id, time))?;
        let mut buf = [0u8; 256];
        loop {
            let n = self.udp.recv(&mut buf)?;
            let packet = &buf[..n];
            let ours = Header::parse(packet).is_some_and(|h| h.kind == datagram::PONG && h.session == self.udp_session);
            let pong = ours.then(|| self.udp_key.open(datagram::TO_PHONE, packet)).flatten().and_then(|plain| datagram::parse_pong(&plain));
            if let Some(pong) = pong.filter(|p| p.0 == id) {
                return Ok(pong);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.send(&json!({"t": "bye"}));
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}
