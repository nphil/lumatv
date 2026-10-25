//! A DTLS 1.2 client for exactly one cipher suite, `TLS_PSK_WITH_AES_128_GCM_SHA256` (0x00A8), which is what Hue
//! Entertainment streaming uses (the bridge, and Bifrost, accept nothing else on UDP port 2100): the PSK identity is
//! the Hue application id and the key is the 16-byte client key from `generateclientkey`.
//!
//! Written against RFC 6347 (DTLS 1.2), RFC 5246 (the PRF, the Finished computation), RFC 4279 (PSK key exchange) and
//! RFC 5288 (AES-GCM records). What it does:
//!
//! - Handshake with the optional HelloVerifyRequest cookie round, handshake fragments reassembled, the last flight
//!   resent on a timer (500 ms, doubling) until `handshake_timeout`.
//! - Records: AES-128-GCM, 8-byte explicit nonce (epoch and sequence number), one record per datagram except the
//!   client's final flight, which goes out as one datagram like every real stack sends it.
//! - Application data out, alerts in. Everything else from the server after the handshake is ignored.
//!
//! What it does not do: other suites, certificates, session resumption, renegotiation, extensions (the ClientHello
//! carries none, which both OpenSSL and mbedTLS servers accept for plain PSK), reordering beyond holding an early
//! encrypted record until the ChangeCipherSpec arrives.

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{AeadInPlace, KeyInit, generic_array::GenericArray};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const VERSION: [u8; 2] = [0xFE, 0xFD];
const SUITE: u16 = 0x00A8;
const CT_CCS: u8 = 20;
const CT_ALERT: u8 = 21;
const CT_HANDSHAKE: u8 = 22;
const CT_APP: u8 = 23;
const HS_CLIENT_HELLO: u8 = 1;
const HS_SERVER_HELLO: u8 = 2;
const HS_HELLO_VERIFY: u8 = 3;
const HS_SERVER_KEY_EXCHANGE: u8 = 12;
const HS_SERVER_HELLO_DONE: u8 = 14;
const HS_CLIENT_KEY_EXCHANGE: u8 = 16;
const HS_FINISHED: u8 = 20;
const RECORD_HEADER: usize = 13;
const HS_HEADER: usize = 12;
const TAG: usize = 16;
const EXPLICIT: usize = 8;
/// A flight is sent as one datagram when it fits; DTLS implementations assume at least 1200 bytes get through.
const FLIGHT_DATAGRAM_MAX: usize = 1200;

#[derive(Clone, Debug)]
pub struct PskConfig {
    /// PSK identity: the Hue application id (the `hue-application-id` header value, 36 characters).
    pub identity: Vec<u8>,
    /// The pre-shared key: the 16 bytes of the `clientkey` (32 hex characters).
    pub psk: Vec<u8>,
    /// Give up on the handshake after this long.
    pub handshake_timeout: Duration,
    /// First retransmission timer; it doubles on every timeout.
    pub first_retransmit: Duration,
}

impl PskConfig {
    pub fn new(identity: &[u8], psk: &[u8]) -> PskConfig {
        PskConfig {
            identity: identity.to_vec(),
            psk: psk.to_vec(),
            handshake_timeout: Duration::from_secs(6),
            first_retransmit: Duration::from_millis(500),
        }
    }
}

#[derive(Debug)]
pub enum DtlsError {
    Io(io::Error),
    /// No (valid) answer within the handshake time: wrong address, port closed, entertainment configuration not
    /// started, wrong key (the server cannot decrypt our Finished and stays silent).
    Timeout,
    /// The peer sent an alert.
    Alert {
        level: u8,
        description: u8,
    },
    /// The peer closed the session (close_notify).
    Closed,
    /// The peer sent something this client does not accept.
    Protocol(&'static str),
}

impl fmt::Display for DtlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DtlsError::Io(e) => write!(f, "dtls i/o: {e}"),
            DtlsError::Timeout => write!(f, "dtls handshake timed out"),
            DtlsError::Alert { level, description } => {
                write!(f, "dtls alert level {level} description {description}")
            }
            DtlsError::Closed => write!(f, "dtls session closed by the peer"),
            DtlsError::Protocol(m) => write!(f, "dtls protocol error: {m}"),
        }
    }
}

impl std::error::Error for DtlsError {}

impl From<io::Error> for DtlsError {
    fn from(e: io::Error) -> Self {
        DtlsError::Io(e)
    }
}

// ---- crypto -----------------------------------------------------------------------------------------------------

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8]) -> HmacSha256 {
    <HmacSha256 as Mac>::new_from_slice(key).expect("hmac takes any key length")
}

/// The TLS 1.2 PRF, `P_SHA256(secret, label + seed)`, filling `out`.
pub(crate) fn prf(secret: &[u8], label: &[u8], seed: &[u8], out: &mut [u8]) {
    let mut a = {
        let mut m = hmac(secret);
        m.update(label);
        m.update(seed);
        m.finalize().into_bytes()
    };
    let mut filled = 0;
    while filled < out.len() {
        let mut m = hmac(secret);
        m.update(&a);
        m.update(label);
        m.update(seed);
        let block = m.finalize().into_bytes();
        let n = (out.len() - filled).min(block.len());
        out[filled..filled + n].copy_from_slice(&block[..n]);
        filled += n;
        let mut m = hmac(secret);
        m.update(&a);
        a = m.finalize().into_bytes();
    }
}

/// RFC 4279 section 2: `uint16 n, n zero bytes, uint16 n, psk` as the pre-master secret of a plain PSK suite.
pub(crate) fn psk_premaster(psk: &[u8]) -> Vec<u8> {
    let n = psk.len() as u16;
    let mut v = Vec::with_capacity(4 + 2 * psk.len());
    v.extend_from_slice(&n.to_be_bytes());
    v.resize(2 + psk.len(), 0);
    v.extend_from_slice(&n.to_be_bytes());
    v.extend_from_slice(psk);
    v
}

pub(crate) fn master_secret(
    psk: &[u8],
    client_random: &[u8; 32],
    server_random: &[u8; 32],
) -> [u8; 48] {
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(client_random);
    seed[32..].copy_from_slice(server_random);
    let mut m = [0u8; 48];
    prf(&psk_premaster(psk), b"master secret", &seed, &mut m);
    m
}

pub(crate) struct Keys {
    pub client: Gcm,
    pub server: Gcm,
}

pub(crate) fn derive_keys(
    master: &[u8; 48],
    client_random: &[u8; 32],
    server_random: &[u8; 32],
) -> Keys {
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(server_random);
    seed[32..].copy_from_slice(client_random);
    let mut kb = [0u8; 40];
    prf(master, b"key expansion", &seed, &mut kb);
    Keys {
        client: Gcm::new(&kb[0..16], &kb[32..36]),
        server: Gcm::new(&kb[16..32], &kb[36..40]),
    }
}

pub(crate) fn verify_data(master: &[u8; 48], label: &[u8], transcript: &[u8]) -> [u8; 12] {
    let hash = Sha256::digest(transcript);
    let mut v = [0u8; 12];
    prf(master, label, &hash, &mut v);
    v
}

/// One direction's AES-128-GCM record protection.
pub(crate) struct Gcm {
    cipher: Aes128Gcm,
    salt: [u8; 4],
}

impl Gcm {
    fn new(key: &[u8], salt: &[u8]) -> Gcm {
        Gcm {
            cipher: Aes128Gcm::new(GenericArray::from_slice(key)),
            salt: salt.try_into().expect("4-byte salt"),
        }
    }

    fn nonce_and_aad(
        &self,
        ct: u8,
        epoch: u16,
        seq: u64,
        plain_len: usize,
    ) -> ([u8; 12], [u8; 13], [u8; 8]) {
        let mut explicit = [0u8; 8];
        explicit[..2].copy_from_slice(&epoch.to_be_bytes());
        explicit[2..].copy_from_slice(&seq.to_be_bytes()[2..]);
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.salt);
        nonce[4..].copy_from_slice(&explicit);
        let mut aad = [0u8; 13];
        aad[..8].copy_from_slice(&explicit);
        aad[8] = ct;
        aad[9..11].copy_from_slice(&VERSION);
        aad[11..].copy_from_slice(&(plain_len as u16).to_be_bytes());
        (nonce, aad, explicit)
    }

    /// The record fragment: explicit nonce, ciphertext, tag.
    pub(crate) fn seal(&self, ct: u8, epoch: u16, seq: u64, plain: &[u8]) -> Vec<u8> {
        let (nonce, aad, explicit) = self.nonce_and_aad(ct, epoch, seq, plain.len());
        let mut buf = plain.to_vec();
        let tag = self
            .cipher
            .encrypt_in_place_detached(GenericArray::from_slice(&nonce), &aad, &mut buf)
            .expect("gcm encrypt");
        let mut out = Vec::with_capacity(EXPLICIT + buf.len() + TAG);
        out.extend_from_slice(&explicit);
        out.extend_from_slice(&buf);
        out.extend_from_slice(&tag);
        out
    }

    pub(crate) fn open(&self, ct: u8, epoch: u16, seq: u64, frag: &[u8]) -> Option<Vec<u8>> {
        if frag.len() < EXPLICIT + TAG {
            return None;
        }
        let plain_len = frag.len() - EXPLICIT - TAG;
        let (_, aad, _) = self.nonce_and_aad(ct, epoch, seq, plain_len);
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.salt);
        nonce[4..].copy_from_slice(&frag[..EXPLICIT]);
        let mut buf = frag[EXPLICIT..EXPLICIT + plain_len].to_vec();
        let tag = GenericArray::from_slice(&frag[EXPLICIT + plain_len..]);
        self.cipher
            .decrypt_in_place_detached(GenericArray::from_slice(&nonce), &aad, &mut buf, tag)
            .ok()?;
        Some(buf)
    }
}

// ---- wire format ------------------------------------------------------------------------------------------------

pub(crate) fn record(ct: u8, epoch: u16, seq: u64, fragment: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(RECORD_HEADER + fragment.len());
    r.push(ct);
    r.extend_from_slice(&VERSION);
    r.extend_from_slice(&epoch.to_be_bytes());
    r.extend_from_slice(&seq.to_be_bytes()[2..]);
    r.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
    r.extend_from_slice(fragment);
    r
}

#[derive(Clone, Debug)]
pub(crate) struct RecordHeader {
    pub ct: u8,
    pub epoch: u16,
    pub seq: u64,
}

/// Splits a datagram into its records; stops at the first malformed one.
pub(crate) fn split_records(d: &[u8]) -> Vec<(RecordHeader, &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + RECORD_HEADER <= d.len() {
        let len = usize::from(u16::from_be_bytes([d[p + 11], d[p + 12]]));
        if d[p + 1] != VERSION[0] || p + RECORD_HEADER + len > d.len() {
            break;
        }
        let mut s = [0u8; 8];
        s[2..].copy_from_slice(&d[p + 5..p + 11]);
        out.push((
            RecordHeader {
                ct: d[p],
                epoch: u16::from_be_bytes([d[p + 3], d[p + 4]]),
                seq: u64::from_be_bytes(s),
            },
            &d[p + RECORD_HEADER..p + RECORD_HEADER + len],
        ));
        p += RECORD_HEADER + len;
    }
    out
}

/// A whole (unfragmented) handshake message with its 12-byte header, as it enters the transcript.
pub(crate) fn hs_message(msg_type: u8, msg_seq: u16, body: &[u8]) -> Vec<u8> {
    let len = (body.len() as u32).to_be_bytes();
    let mut m = Vec::with_capacity(HS_HEADER + body.len());
    m.push(msg_type);
    m.extend_from_slice(&len[1..]);
    m.extend_from_slice(&msg_seq.to_be_bytes());
    m.extend_from_slice(&[0, 0, 0]); // fragment offset
    m.extend_from_slice(&len[1..]); // fragment length
    m.extend_from_slice(body);
    m
}

/// Reassembles fragmented handshake messages and releases them in message_seq order.
#[derive(Default)]
pub(crate) struct Reassembler {
    next: u16,
    pending: Vec<Partial>,
}

struct Partial {
    seq: u16,
    msg_type: u8,
    len: usize,
    data: Vec<u8>,
    have: Vec<bool>,
}

pub(crate) struct HsMessage {
    pub msg_type: u8,
    pub seq: u16,
    pub body: Vec<u8>,
}

impl Reassembler {
    /// Takes one record's handshake payload (one or more fragments); returns the messages that are now complete and next in line.
    pub(crate) fn push(&mut self, mut p: &[u8]) -> Vec<HsMessage> {
        while p.len() >= HS_HEADER {
            let msg_type = p[0];
            let len = (usize::from(p[1]) << 16) | (usize::from(p[2]) << 8) | usize::from(p[3]);
            let seq = u16::from_be_bytes([p[4], p[5]]);
            let off = (usize::from(p[6]) << 16) | (usize::from(p[7]) << 8) | usize::from(p[8]);
            let flen = (usize::from(p[9]) << 16) | (usize::from(p[10]) << 8) | usize::from(p[11]);
            if p.len() < HS_HEADER + flen || off + flen > len || len > 16_384 {
                break;
            }
            let frag = &p[HS_HEADER..HS_HEADER + flen];
            p = &p[HS_HEADER + flen..];
            if seq < self.next {
                continue; // a retransmission of something already handled
            }
            let part = match self.pending.iter_mut().find(|q| q.seq == seq) {
                Some(q) => q,
                None => {
                    self.pending.push(Partial {
                        seq,
                        msg_type,
                        len,
                        data: vec![0; len],
                        have: vec![false; len],
                    });
                    self.pending.last_mut().expect("just pushed")
                }
            };
            if part.len != len || part.msg_type != msg_type {
                continue;
            }
            part.data[off..off + flen].copy_from_slice(frag);
            part.have[off..off + flen]
                .iter_mut()
                .for_each(|h| *h = true);
        }
        let mut out = Vec::new();
        while let Some(i) = self
            .pending
            .iter()
            .position(|q| q.seq == self.next && q.have.iter().all(|&h| h))
        {
            let q = self.pending.swap_remove(i);
            out.push(HsMessage {
                msg_type: q.msg_type,
                seq: q.seq,
                body: q.data,
            });
            self.next += 1;
        }
        out
    }

    /// Message sequence the next in-order server message must carry.
    #[cfg(test)]
    pub(crate) fn next_seq(&self) -> u16 {
        self.next
    }
}

// ---- the client -------------------------------------------------------------------------------------------------

enum Item {
    Handshake {
        msg_type: u8,
        msg_seq: u16,
        body: Vec<u8>,
        encrypted: bool,
    },
    ChangeCipherSpec,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum State {
    Hello,
    ServerHello,
    ServerHelloDone,
    ServerFinished,
}

pub struct PskClient {
    socket: UdpSocket,
    keys: Option<Keys>,
    seq0: u64,
    seq1: u64,
}

impl PskClient {
    /// Connects the UDP socket to `addr` and runs the handshake. The socket is left in blocking mode with no timeout.
    pub fn connect(addr: SocketAddr, cfg: &PskConfig) -> Result<PskClient, DtlsError> {
        let bind: SocketAddr = if addr.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            ([0u16; 8], 0).into()
        };
        let socket = UdpSocket::bind(bind)?;
        socket.connect(addr)?;
        let mut c = PskClient {
            socket,
            keys: None,
            seq0: 0,
            seq1: 0,
        };
        c.handshake(cfg)?;
        Ok(c)
    }

    /// Encodes a flight with fresh record numbers (a retransmission is a new record to the receiver) and sends it:
    /// one datagram when it fits, else one per record.
    fn send_flight(&mut self, flight: &[Item]) -> io::Result<()> {
        let (mut seq0, mut seq1) = (self.seq0, self.seq1);
        let mut records = Vec::new();
        for item in flight {
            match item {
                Item::Handshake {
                    msg_type,
                    msg_seq,
                    body,
                    encrypted: false,
                } => {
                    records.push(record(
                        CT_HANDSHAKE,
                        0,
                        seq0,
                        &hs_message(*msg_type, *msg_seq, body),
                    ));
                    seq0 += 1;
                }
                Item::Handshake {
                    msg_type,
                    msg_seq,
                    body,
                    encrypted: true,
                } => {
                    let keys = self
                        .keys
                        .as_ref()
                        .expect("keys exist before an encrypted record");
                    let frag = keys.client.seal(
                        CT_HANDSHAKE,
                        1,
                        seq1,
                        &hs_message(*msg_type, *msg_seq, body),
                    );
                    records.push(record(CT_HANDSHAKE, 1, seq1, &frag));
                    seq1 += 1;
                }
                Item::ChangeCipherSpec => {
                    records.push(record(CT_CCS, 0, seq0, &[1]));
                    seq0 += 1;
                }
            }
        }
        (self.seq0, self.seq1) = (seq0, seq1);
        let total: usize = records.iter().map(Vec::len).sum();
        if total <= FLIGHT_DATAGRAM_MAX {
            self.socket.send(&records.concat())?;
        } else {
            for r in &records {
                self.socket.send(r)?;
            }
        }
        Ok(())
    }

    fn handshake(&mut self, cfg: &PskConfig) -> Result<(), DtlsError> {
        let mut client_random = [0u8; 32];
        getrandom::getrandom(&mut client_random)
            .map_err(|_| DtlsError::Protocol("no random source"))?;
        let mut server_random = [0u8; 32];
        let mut transcript: Vec<u8> = Vec::new();
        let mut reasm = Reassembler::default();
        let mut master = [0u8; 48];
        let mut tx_msg_seq: u16 = 0;

        let hello_body = |cookie: &[u8]| {
            let mut b = Vec::new();
            b.extend_from_slice(&VERSION);
            b.extend_from_slice(&client_random);
            b.push(0); // session id
            b.push(cookie.len() as u8);
            b.extend_from_slice(cookie);
            b.extend_from_slice(&2u16.to_be_bytes());
            b.extend_from_slice(&SUITE.to_be_bytes());
            b.extend_from_slice(&[1, 0]); // compression: null only
            b
        };
        let body = hello_body(&[]);
        transcript.extend_from_slice(&hs_message(HS_CLIENT_HELLO, tx_msg_seq, &body));
        let mut flight = vec![Item::Handshake {
            msg_type: HS_CLIENT_HELLO,
            msg_seq: tx_msg_seq,
            body,
            encrypted: false,
        }];
        tx_msg_seq += 1;
        self.send_flight(&flight)?;

        let mut state = State::Hello;
        let started = Instant::now();
        let mut timer = cfg.first_retransmit;
        let mut timer_start = Instant::now();
        let mut got_ccs = false;
        let mut early: Vec<(RecordHeader, Vec<u8>)> = Vec::new();
        let mut buf = vec![0u8; 4096];

        loop {
            if started.elapsed() >= cfg.handshake_timeout {
                return Err(DtlsError::Timeout);
            }
            let left = timer.saturating_sub(timer_start.elapsed());
            if left.is_zero() {
                self.send_flight(&flight)?;
                timer = (timer * 2).min(Duration::from_secs(4));
                timer_start = Instant::now();
                continue;
            }
            self.socket.set_read_timeout(Some(left))?;
            let n = match self.socket.recv(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let mut records: Vec<(RecordHeader, Vec<u8>)> = split_records(&buf[..n])
                .into_iter()
                .map(|(h, f)| (h, f.to_vec()))
                .collect();
            let mut i = 0;
            while i < records.len() {
                let (h, frag) = records[i].clone();
                i += 1;
                let mut payloads: Vec<Vec<u8>> = Vec::new();
                match (h.ct, h.epoch) {
                    (CT_ALERT, 0) if frag.len() >= 2 => return Err(alert_error(frag[0], frag[1])),
                    (CT_HANDSHAKE, 0) => payloads.push(frag),
                    (CT_CCS, 0) => {
                        if state == State::ServerFinished && !got_ccs {
                            got_ccs = true;
                            // Encrypted records that overtook the ChangeCipherSpec are handled now.
                            records.append(&mut early);
                        }
                    }
                    (_, 1) if !got_ccs => {
                        if early.len() < 8 {
                            early.push((h, frag));
                        }
                    }
                    (_, 1) => {
                        let keys = self
                            .keys
                            .as_ref()
                            .ok_or(DtlsError::Protocol("encrypted record before keys"))?;
                        let Some(plain) = keys.server.open(h.ct, h.epoch, h.seq, &frag) else {
                            continue;
                        };
                        match h.ct {
                            CT_HANDSHAKE => payloads.push(plain),
                            CT_ALERT if plain.len() >= 2 => {
                                return Err(alert_error(plain[0], plain[1]));
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }

                for payload in payloads {
                    // The server's Finished is epoch 1 but its message_seq follows the plaintext messages, so one
                    // reassembler serves both epochs.
                    for m in reasm.push(&payload) {
                        match (state, m.msg_type) {
                            (State::Hello, HS_HELLO_VERIFY) => {
                                if m.body.len() < 3 || usize::from(m.body[2]) + 3 > m.body.len() {
                                    return Err(DtlsError::Protocol(
                                        "malformed HelloVerifyRequest",
                                    ));
                                }
                                let cookie = m.body[3..3 + usize::from(m.body[2])].to_vec();
                                // The first ClientHello and this message are not part of the Finished hash.
                                transcript.clear();
                                let body = hello_body(&cookie);
                                transcript.extend_from_slice(&hs_message(
                                    HS_CLIENT_HELLO,
                                    tx_msg_seq,
                                    &body,
                                ));
                                flight = vec![Item::Handshake {
                                    msg_type: HS_CLIENT_HELLO,
                                    msg_seq: tx_msg_seq,
                                    body,
                                    encrypted: false,
                                }];
                                tx_msg_seq += 1;
                                self.send_flight(&flight)?;
                                timer = cfg.first_retransmit;
                                timer_start = Instant::now();
                                state = State::ServerHello;
                            }
                            (State::Hello | State::ServerHello, HS_SERVER_HELLO) => {
                                let sid_at = 2 + 32;
                                if m.body.len() < sid_at + 1 {
                                    return Err(DtlsError::Protocol("short ServerHello"));
                                }
                                let suite_at = sid_at + 1 + usize::from(m.body[sid_at]);
                                if m.body.len() < suite_at + 3 {
                                    return Err(DtlsError::Protocol("short ServerHello"));
                                }
                                if u16::from_be_bytes([m.body[suite_at], m.body[suite_at + 1]])
                                    != SUITE
                                {
                                    return Err(DtlsError::Protocol(
                                        "server chose another cipher suite",
                                    ));
                                }
                                server_random.copy_from_slice(&m.body[2..34]);
                                transcript
                                    .extend_from_slice(&hs_message(m.msg_type, m.seq, &m.body));
                                state = State::ServerHelloDone;
                            }
                            (State::ServerHelloDone, HS_SERVER_KEY_EXCHANGE) => {
                                // A PSK identity hint, which this client has no use for; it still enters the hash.
                                transcript
                                    .extend_from_slice(&hs_message(m.msg_type, m.seq, &m.body));
                            }
                            (State::ServerHelloDone, HS_SERVER_HELLO_DONE) => {
                                transcript
                                    .extend_from_slice(&hs_message(m.msg_type, m.seq, &m.body));
                                let mut cke = Vec::with_capacity(2 + cfg.identity.len());
                                cke.extend_from_slice(&(cfg.identity.len() as u16).to_be_bytes());
                                cke.extend_from_slice(&cfg.identity);
                                let (cke_seq, fin_seq) = (tx_msg_seq, tx_msg_seq + 1);
                                tx_msg_seq += 2;
                                transcript.extend_from_slice(&hs_message(
                                    HS_CLIENT_KEY_EXCHANGE,
                                    cke_seq,
                                    &cke,
                                ));
                                master = master_secret(&cfg.psk, &client_random, &server_random);
                                self.keys =
                                    Some(derive_keys(&master, &client_random, &server_random));
                                let fin = verify_data(&master, b"client finished", &transcript);
                                // The server's Finished hash covers ours.
                                transcript.extend_from_slice(&hs_message(
                                    HS_FINISHED,
                                    fin_seq,
                                    &fin,
                                ));
                                flight = vec![
                                    Item::Handshake {
                                        msg_type: HS_CLIENT_KEY_EXCHANGE,
                                        msg_seq: cke_seq,
                                        body: cke,
                                        encrypted: false,
                                    },
                                    Item::ChangeCipherSpec,
                                    Item::Handshake {
                                        msg_type: HS_FINISHED,
                                        msg_seq: fin_seq,
                                        body: fin.to_vec(),
                                        encrypted: true,
                                    },
                                ];
                                self.send_flight(&flight)?;
                                timer = cfg.first_retransmit;
                                timer_start = Instant::now();
                                state = State::ServerFinished;
                            }
                            (State::ServerFinished, HS_FINISHED) => {
                                let expected =
                                    verify_data(&master, b"server finished", &transcript);
                                if !ct_eq(&m.body, &expected) {
                                    return Err(DtlsError::Protocol(
                                        "server Finished does not verify",
                                    ));
                                }
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Sends one datagram of application data.
    pub fn send(&mut self, data: &[u8]) -> Result<(), DtlsError> {
        let keys = self
            .keys
            .as_ref()
            .ok_or(DtlsError::Protocol("not connected"))?;
        let frag = keys.client.seal(CT_APP, 1, self.seq1, data);
        let r = record(CT_APP, 1, self.seq1, &frag);
        self.seq1 += 1;
        self.socket.send(&r)?;
        Ok(())
    }

    /// Looks for an alert from the peer without blocking: `Err(Closed)` after a close_notify, `Err(Alert)` after a
    /// fatal one. Application data and anything undecryptable is dropped.
    pub fn poll(&mut self) -> Result<(), DtlsError> {
        self.socket.set_nonblocking(true)?;
        let mut buf = [0u8; 2048];
        let mut outcome = Ok(());
        loop {
            match self.socket.recv(&mut buf) {
                Ok(n) => {
                    if let Some(e) = self.alert_in(&buf[..n]) {
                        outcome = Err(e);
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    outcome = Err(e.into());
                    break;
                }
            }
        }
        self.socket.set_nonblocking(false)?;
        outcome
    }

    fn alert_in(&self, datagram: &[u8]) -> Option<DtlsError> {
        let keys = self.keys.as_ref()?;
        split_records(datagram)
            .into_iter()
            .filter(|(h, _)| h.ct == CT_ALERT && h.epoch == 1)
            .filter_map(|(h, frag)| keys.server.open(h.ct, h.epoch, h.seq, frag))
            .find(|p| p.len() >= 2)
            .map(|p| alert_error(p[0], p[1]))
    }

    /// Sends close_notify. The socket closes when the client is dropped.
    pub fn close(&mut self) {
        if let Some(keys) = self.keys.as_ref() {
            let frag = keys.client.seal(CT_ALERT, 1, self.seq1, &[1, 0]);
            let r = record(CT_ALERT, 1, self.seq1, &frag);
            self.seq1 += 1;
            let _ = self.socket.send(&r);
        }
    }
}

fn alert_error(level: u8, description: u8) -> DtlsError {
    if description == 0 {
        DtlsError::Closed
    } else {
        DtlsError::Alert { level, description }
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod testserver;
