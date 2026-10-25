//! Hue Entertainment streaming, API v2, to a Hue bridge or to Bifrost: the `HueStream` packet and the session that
//! carries it over DTLS ([`crate::dtls`]).
//!
//! The helper does three things around a stream, of which only the middle one is in this module:
//!
//! 1. REST, over HTTPS to the bridge (a self-signed certificate): `PUT /clip/v2/resource/entertainment_configuration/<id>`
//!    with [`start_request`] before streaming and [`stop_request`] after; header `hue-application-key: <username>`.
//!    The configuration lists the channels (up to 20) and where each light sits.
//! 2. DTLS to UDP port 2100 with PSK identity = the application id (`hue-application-id`, from `GET /auth/v1`) and PSK =
//!    the `clientkey` the bridge returned when the application key was created (32 hex characters, 16 bytes).
//! 3. [`Session::send`] about 20 to 25 times a second. The bridge stops the stream after about 10 s of silence; Zigbee
//!    lights take roughly 25 updates a second whatever the stream rate. Bifrost forwards at most `streaming_fps` frames
//!    a second (default 20) and smooths between them for the rate it sees, so it looks best when the stream rate
//!    equals that setting.
//!
//! Bifrost (read from its source, `src/server/entertainment.rs` and `crates/hue/src/stream.rs`): the DTLS server is
//! OpenSSL's with a PSK callback that ignores the identity and always answers with the fixed key `BifrostHueTlsKey`
//! ([`BIFROST_CLIENT_KEY_HEX`]); it accepts HueStream v1 and v2, RGB and xy; it reads each datagram into a 1024-byte
//! buffer; it allows 5 s for the handshake and 10 s of silence; the first frame of a session is used to find the
//! entertainment configuration and is not forwarded; the configuration must exist in Bifrost.
//!
//! Packet (v2), all integers big endian:
//!
//! ```text
//!   "HueStream" (9)  version 0x02 0x00  sequence (1)  0x00 0x00  colour space (1: 0 RGB, 1 xy+brightness)  0x00
//!   entertainment configuration id (36 ASCII bytes, the UUID)
//!   per channel: channel id (1)  three u16 (R G B)
//! ```

use crate::dtls::{DtlsError, PskClient, PskConfig};
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

pub const PORT: u16 = 2100;
pub const MAX_CHANNELS: usize = 20;
const MAGIC: &[u8; 9] = b"HueStream";
const HEADER: usize = 16;
pub const CONFIG_ID_LEN: usize = 36;

/// The 16-byte key Bifrost uses for every entertainment stream (`BifrostHueTlsKey`), as the `clientkey` text it hands out.
pub const BIFROST_CLIENT_KEY_HEX: &str = "426966726f7374487565546c734b6579";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Rgb = 0,
    XyBrightness = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamError {
    ConfigId,
    TooManyChannels(usize),
    ClientKey,
    Dtls(String),
    Closed,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamError::ConfigId => write!(
                f,
                "the entertainment configuration id must be a 36 character UUID"
            ),
            StreamError::TooManyChannels(n) => write!(
                f,
                "{n} channels; an entertainment stream carries at most {MAX_CHANNELS}"
            ),
            StreamError::ClientKey => write!(f, "the client key must be 32 hex characters"),
            StreamError::Dtls(m) => write!(f, "{m}"),
            StreamError::Closed => write!(f, "the bridge ended the stream"),
        }
    }
}

impl std::error::Error for StreamError {}

fn is_uuid(s: &str) -> bool {
    s.len() == CONFIG_ID_LEN
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// One HueStream v2 packet. Channels are `(channel id, [r, g, b])` as 16-bit values.
pub fn encode(
    seq: u8,
    space: ColorSpace,
    config_id: &str,
    channels: &[(u8, [u16; 3])],
) -> Result<Vec<u8>, StreamError> {
    if !is_uuid(config_id) {
        return Err(StreamError::ConfigId);
    }
    if channels.len() > MAX_CHANNELS {
        return Err(StreamError::TooManyChannels(channels.len()));
    }
    let mut p = Vec::with_capacity(HEADER + CONFIG_ID_LEN + channels.len() * 7);
    p.extend_from_slice(MAGIC);
    p.extend_from_slice(&[0x02, 0x00, seq, 0x00, 0x00, space as u8, 0x00]);
    p.extend_from_slice(config_id.as_bytes());
    for (id, c) in channels {
        p.push(*id);
        for v in c {
            p.extend_from_slice(&v.to_be_bytes());
        }
    }
    Ok(p)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub seq: u8,
    pub space: ColorSpace,
    pub config_id: String,
    pub channels: Vec<(u8, [u16; 3])>,
}

/// Parses a v2 packet; what the bridge side does with it, here for the fakes the tests talk to.
pub fn parse(p: &[u8]) -> Option<Parsed> {
    if p.len() < HEADER + CONFIG_ID_LEN || &p[..9] != MAGIC || p[9] != 2 || p[10] != 0 {
        return None;
    }
    let space = match p[14] {
        0 => ColorSpace::Rgb,
        1 => ColorSpace::XyBrightness,
        _ => return None,
    };
    let rest = &p[HEADER + CONFIG_ID_LEN..];
    if rest.len() % 7 != 0 {
        return None;
    }
    let channels = rest
        .chunks_exact(7)
        .map(|c| {
            (
                c[0],
                [
                    u16::from_be_bytes([c[1], c[2]]),
                    u16::from_be_bytes([c[3], c[4]]),
                    u16::from_be_bytes([c[5], c[6]]),
                ],
            )
        })
        .collect();
    Some(Parsed {
        seq: p[11],
        space,
        config_id: String::from_utf8(p[HEADER..HEADER + CONFIG_ID_LEN].to_vec()).ok()?,
        channels,
    })
}

/// The 16 bytes behind a `clientkey`.
pub fn decode_client_key(hex: &str) -> Result<Vec<u8>, StreamError> {
    let h = hex.trim();
    if h.len() != 32 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StreamError::ClientKey);
    }
    Ok((0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).expect("checked hex"))
        .collect())
}

/// A REST request the helper sends over HTTPS (the bridge's certificate is self-signed): method, path, JSON body.
pub struct Request {
    pub method: &'static str,
    pub path: String,
    pub body: &'static str,
}

pub fn start_request(config_id: &str) -> Request {
    Request {
        method: "PUT",
        path: format!("/clip/v2/resource/entertainment_configuration/{config_id}"),
        body: r#"{"action":"start"}"#,
    }
}

pub fn stop_request(config_id: &str) -> Request {
    Request {
        method: "PUT",
        path: format!("/clip/v2/resource/entertainment_configuration/{config_id}"),
        body: r#"{"action":"stop"}"#,
    }
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Bridge (or Bifrost) address, port 2100.
    pub addr: SocketAddr,
    /// `hue-application-id`.
    pub app_id: String,
    /// The 32 hex characters of the client key.
    pub client_key: String,
    /// The entertainment configuration's UUID, already started over REST.
    pub config_id: String,
    pub colour_space: ColorSpace,
    pub handshake_timeout: Duration,
}

impl SessionConfig {
    pub fn new(addr: SocketAddr, app_id: &str, client_key: &str, config_id: &str) -> SessionConfig {
        SessionConfig {
            addr,
            app_id: app_id.to_string(),
            client_key: client_key.to_string(),
            config_id: config_id.to_string(),
            colour_space: ColorSpace::Rgb,
            handshake_timeout: Duration::from_secs(6),
        }
    }
}

pub struct Session {
    dtls: PskClient,
    seq: u8,
    config_id: String,
    space: ColorSpace,
}

impl Session {
    pub fn connect(cfg: &SessionConfig) -> Result<Session, StreamError> {
        if !is_uuid(&cfg.config_id) {
            return Err(StreamError::ConfigId);
        }
        let psk = decode_client_key(&cfg.client_key)?;
        let mut d = PskConfig::new(cfg.app_id.as_bytes(), &psk);
        d.handshake_timeout = cfg.handshake_timeout;
        let dtls =
            PskClient::connect(cfg.addr, &d).map_err(|e| StreamError::Dtls(e.to_string()))?;
        Ok(Session {
            dtls,
            seq: 0,
            config_id: cfg.config_id.clone(),
            space: cfg.colour_space,
        })
    }

    /// One update for all channels. Call at the stream rate; a still picture still needs the calls (keep-alive).
    pub fn send(&mut self, channels: &[(u8, [u16; 3])]) -> Result<(), StreamError> {
        let p = encode(self.seq, self.space, &self.config_id, channels)?;
        self.seq = self.seq.wrapping_add(1);
        self.dtls
            .send(&p)
            .map_err(|e| StreamError::Dtls(e.to_string()))
    }

    /// Whether the bridge has closed the session (it does when the entertainment configuration is stopped).
    pub fn poll(&mut self) -> Result<(), StreamError> {
        match self.dtls.poll() {
            Ok(()) => Ok(()),
            Err(DtlsError::Closed) => Err(StreamError::Closed),
            Err(e) => Err(StreamError::Dtls(e.to_string())),
        }
    }

    pub fn close(&mut self) {
        self.dtls.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtls::testserver::{PSK, ServerOpts, hex, spawn_server};

    const CFG: &str = "7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10";

    #[test]
    fn packet_bytes_match_the_v2_layout() {
        let p = encode(
            7,
            ColorSpace::Rgb,
            CFG,
            &[(0, [0xFFFF, 0, 0x1234]), (5, [1, 2, 3])],
        )
        .unwrap();
        assert_eq!(&p[..9], b"HueStream");
        assert_eq!(&p[9..16], &[2, 0, 7, 0, 0, 0, 0]);
        assert_eq!(&p[16..52], CFG.as_bytes());
        assert_eq!(&p[52..59], &[0, 0xFF, 0xFF, 0, 0, 0x12, 0x34]);
        assert_eq!(&p[59..66], &[5, 0, 1, 0, 2, 0, 3]);
        assert_eq!(p.len(), 66);
        let x = encode(0, ColorSpace::XyBrightness, CFG, &[]).unwrap();
        assert_eq!(x[14], 1);
        assert_eq!(x.len(), 52);
    }

    #[test]
    fn parse_round_trips_and_refuses_junk() {
        let ch = vec![(0u8, [1u16, 2, 3]), (19, [65535, 0, 32768])];
        let p = encode(250, ColorSpace::Rgb, CFG, &ch).unwrap();
        assert_eq!(
            parse(&p),
            Some(Parsed {
                seq: 250,
                space: ColorSpace::Rgb,
                config_id: CFG.to_string(),
                channels: ch
            })
        );
        assert_eq!(parse(&p[..p.len() - 1]), None);
        let mut v1 = p.clone();
        v1[9] = 1;
        assert_eq!(parse(&v1), None);
        assert_eq!(parse(b"HueStream"), None);
        let mut bad_space = p.clone();
        bad_space[14] = 9;
        assert_eq!(parse(&bad_space), None);
    }

    #[test]
    fn encode_checks_its_inputs() {
        assert_eq!(
            encode(0, ColorSpace::Rgb, "not-a-uuid", &[]),
            Err(StreamError::ConfigId)
        );
        assert_eq!(
            encode(
                0,
                ColorSpace::Rgb,
                "7c6ab1f2_3e44_4a5e_9d1b_0c8f6a2d9e10",
                &[]
            ),
            Err(StreamError::ConfigId)
        );
        let many: Vec<(u8, [u16; 3])> = (0..21).map(|i| (i, [0; 3])).collect();
        assert_eq!(
            encode(0, ColorSpace::Rgb, CFG, &many),
            Err(StreamError::TooManyChannels(21))
        );
        assert!(encode(0, ColorSpace::Rgb, CFG, &many[..20]).is_ok());
    }

    #[test]
    fn client_key_is_sixteen_bytes_of_hex() {
        assert_eq!(
            decode_client_key("00112233445566778899AABBCCDDEEFF").unwrap(),
            hex(PSK)
        );
        assert_eq!(
            decode_client_key(" 00112233445566778899aabbccddeeff\n").unwrap(),
            hex(PSK)
        );
        assert_eq!(decode_client_key("0011"), Err(StreamError::ClientKey));
        assert_eq!(
            decode_client_key("zz112233445566778899aabbccddeeff"),
            Err(StreamError::ClientKey)
        );
    }

    #[test]
    fn bifrosts_fixed_key_is_the_text_it_uses() {
        assert_eq!(
            decode_client_key(BIFROST_CLIENT_KEY_HEX).unwrap(),
            b"BifrostHueTlsKey"
        );
    }

    #[test]
    fn rest_requests_are_the_v2_start_and_stop_actions() {
        let s = start_request(CFG);
        assert_eq!(
            (s.method, s.path.as_str(), s.body),
            (
                "PUT",
                "/clip/v2/resource/entertainment_configuration/7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10",
                r#"{"action":"start"}"#
            )
        );
        assert_eq!(stop_request(CFG).body, r#"{"action":"stop"}"#);
    }

    #[test]
    fn a_session_streams_frames_to_a_fake_bridge_over_dtls() {
        let o = ServerOpts {
            identity: b"app-id-for-the-test".to_vec(),
            hello_verify: true,
            ..ServerOpts::new()
        };
        let bridge = spawn_server(o.clone());
        let mut cfg = SessionConfig::new(bridge.addr, "app-id-for-the-test", PSK, CFG);
        cfg.handshake_timeout = Duration::from_secs(4);
        let mut s = Session::connect(&cfg).expect("session");
        for i in 0..70u32 {
            s.send(&[(0, [(i * 900) as u16, 0, 65535]), (1, [0, 0, 0])])
                .unwrap();
        }
        let mut seqs = Vec::new();
        for i in 0..70u32 {
            let raw = bridge
                .rx
                .recv_timeout(Duration::from_secs(3))
                .expect("a frame at the bridge");
            let p = parse(&raw).expect("a HueStream v2 packet");
            assert_eq!(p.config_id, CFG);
            assert_eq!(
                p.channels,
                vec![(0, [(i * 900) as u16, 0, 65535]), (1, [0, 0, 0])]
            );
            seqs.push(p.seq);
        }
        assert_eq!(&seqs[..3], &[0, 1, 2]);
        assert_eq!(seqs[69], 69, "sequence counts per frame");
        s.close();
        assert_eq!(
            bridge.rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            b"<close_notify>"
        );
    }

    #[test]
    fn a_wrong_key_or_bad_input_fails_without_traffic_to_anything_else() {
        let o = ServerOpts::new();
        let bridge = spawn_server(o);
        let mut cfg = SessionConfig::new(bridge.addr, "x", "00112233445566778899aabbccddee", CFG);
        assert_eq!(Session::connect(&cfg).err(), Some(StreamError::ClientKey));
        cfg.client_key = PSK.to_string();
        cfg.config_id = "nope".into();
        assert_eq!(Session::connect(&cfg).err(), Some(StreamError::ConfigId));
    }
}
