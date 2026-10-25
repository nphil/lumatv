//! An in-process DTLS 1.2 PSK server for the tests: the server side of RFC 6347 written with the crate's own
//! primitives, with switches for the situations a real network produces (cookie round, fragmented flights, a lost
//! client flight, a reordered ChangeCipherSpec, a close).

use super::*;
use std::sync::mpsc;
use std::thread;

pub(crate) const PSK: &str = "00112233445566778899aabbccddeeff";

pub(crate) fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

pub(crate) fn fragment(
    msg_type: u8,
    msg_seq: u16,
    total: &[u8],
    off: usize,
    len: usize,
) -> Vec<u8> {
    let t = (total.len() as u32).to_be_bytes();
    let o = (off as u32).to_be_bytes();
    let l = (len as u32).to_be_bytes();
    let mut f = vec![msg_type, t[1], t[2], t[3]];
    f.extend_from_slice(&msg_seq.to_be_bytes());
    f.extend_from_slice(&o[1..]);
    f.extend_from_slice(&l[1..]);
    f.extend_from_slice(&total[off..off + len]);
    f
}

// ---- an in-process server ---------------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ServerOpts {
    pub psk: Vec<u8>,
    pub identity: Vec<u8>,
    pub hello_verify: bool,
    /// Ignore the first datagram that carries the client's key exchange, as if it was lost.
    pub drop_first_client_flight: bool,
    /// Send the ServerHello as two handshake fragments.
    pub fragment_server_hello: bool,
    /// Send an unsolicited PSK identity hint (ServerKeyExchange) before ServerHelloDone, as mbedTLS does with a hint set.
    pub key_exchange_hint: bool,
    /// Put the encrypted Finished before the ChangeCipherSpec, in separate datagrams.
    pub finished_before_ccs: bool,
    /// Close the session with close_notify this long after the handshake.
    pub close_after: Option<Duration>,
}

impl ServerOpts {
    pub(crate) fn new() -> ServerOpts {
        ServerOpts {
            psk: hex(PSK),
            identity: b"lumatv-test-app".to_vec(),
            hello_verify: false,
            drop_first_client_flight: false,
            fragment_server_hello: false,
            key_exchange_hint: false,
            finished_before_ccs: false,
            close_after: None,
        }
    }
}

pub(crate) struct TestServer {
    pub addr: SocketAddr,
    pub rx: mpsc::Receiver<Vec<u8>>,
    pub seen_flights: mpsc::Receiver<usize>,
}

pub(crate) fn spawn_server(o: ServerOpts) -> TestServer {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    let addr = sock.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let (ftx, seen_flights) = mpsc::channel();
    thread::spawn(move || {
        let _ = serve(sock, o, tx, ftx);
    });
    TestServer {
        addr,
        rx,
        seen_flights,
    }
}

fn serve(
    sock: UdpSocket,
    o: ServerOpts,
    tx: mpsc::Sender<Vec<u8>>,
    flights: mpsc::Sender<usize>,
) -> Option<()> {
    let mut buf = [0u8; 4096];
    let mut seq0 = 0u64;
    let mut seq1 = 0u64;
    let mut msg_seq = 0u16;
    let mut reasm = Reassembler::default();
    let mut transcript = Vec::new();
    let cookie = [0xAAu8; 16];
    let mut peer;
    let client_random: [u8; 32];

    // ClientHello, once more with the cookie when asked for.
    loop {
        let (n, from) = sock.recv_from(&mut buf).ok()?;
        peer = from;
        let mut got = None;
        for (h, f) in split_records(&buf[..n]) {
            if h.ct == CT_HANDSHAKE && h.epoch == 0 {
                for m in reasm.push(f) {
                    if m.msg_type == HS_CLIENT_HELLO {
                        got = Some(m);
                    }
                }
            }
        }
        let Some(m) = got else { continue };
        let sid = usize::from(m.body[34]);
        let cookie_at = 35 + sid;
        let cl = usize::from(m.body[cookie_at]);
        let their_cookie = &m.body[cookie_at + 1..cookie_at + 1 + cl];
        if o.hello_verify && their_cookie != cookie {
            let mut body = VERSION.to_vec();
            body.push(cookie.len() as u8);
            body.extend_from_slice(&cookie);
            sock.send_to(
                &record(
                    CT_HANDSHAKE,
                    0,
                    seq0,
                    &hs_message(HS_HELLO_VERIFY, 0, &body),
                ),
                peer,
            )
            .ok()?;
            seq0 += 1;
            // Answered statelessly: the client's next hello is message 1 and starts the transcript; our next message is 1 too.
            msg_seq = 1;
            continue;
        }
        client_random = m.body[2..34].try_into().ok()?;
        transcript.extend_from_slice(&hs_message(HS_CLIENT_HELLO, m.seq, &m.body));
        break;
    }

    // ServerHello, [ServerKeyExchange], ServerHelloDone.
    let mut server_random = [0u8; 32];
    getrandom::getrandom(&mut server_random).ok()?;
    let mut sh = VERSION.to_vec();
    sh.extend_from_slice(&server_random);
    sh.push(0);
    sh.extend_from_slice(&SUITE.to_be_bytes());
    sh.push(0);
    let mut datagram = Vec::new();
    let sh_msg = hs_message(HS_SERVER_HELLO, msg_seq, &sh);
    transcript.extend_from_slice(&sh_msg);
    if o.fragment_server_hello {
        let half = sh.len() / 2;
        for (off, len) in [(0, half), (half, sh.len() - half)] {
            let f = fragment(HS_SERVER_HELLO, msg_seq, &sh, off, len);
            datagram.extend(record(CT_HANDSHAKE, 0, seq0, &f));
            seq0 += 1;
        }
    } else {
        datagram.extend(record(CT_HANDSHAKE, 0, seq0, &sh_msg));
        seq0 += 1;
    }
    msg_seq += 1;
    if o.key_exchange_hint {
        let hint = [0u8, 4, b'h', b'i', b'n', b't'];
        let m = hs_message(HS_SERVER_KEY_EXCHANGE, msg_seq, &hint);
        transcript.extend_from_slice(&m);
        datagram.extend(record(CT_HANDSHAKE, 0, seq0, &m));
        seq0 += 1;
        msg_seq += 1;
    }
    let done = hs_message(HS_SERVER_HELLO_DONE, msg_seq, &[]);
    transcript.extend_from_slice(&done);
    datagram.extend(record(CT_HANDSHAKE, 0, seq0, &done));
    seq0 += 1;
    msg_seq += 1;
    sock.send_to(&datagram, peer).ok()?;

    // The client's flight: ClientKeyExchange, ChangeCipherSpec, Finished.
    let master = master_secret(&o.psk, &client_random, &server_random);
    let keys = derive_keys(&master, &client_random, &server_random);
    let mut dropped = false;
    let mut got_cke = false;
    let mut flights_seen = 0usize;
    loop {
        let (n, from) = sock.recv_from(&mut buf).ok()?;
        peer = from;
        let recs = split_records(&buf[..n]);
        if recs.iter().any(|(h, f)| {
            h.ct == CT_HANDSHAKE && h.epoch == 0 && f.first() == Some(&HS_CLIENT_KEY_EXCHANGE)
        }) {
            flights_seen += 1;
            let _ = flights.send(flights_seen);
            if o.drop_first_client_flight && !dropped {
                dropped = true; // as if this datagram never arrived
                continue;
            }
        }
        for (h, f) in recs {
            match (h.ct, h.epoch) {
                (CT_HANDSHAKE, 0) => {
                    for m in reasm.push(f) {
                        if m.msg_type == HS_CLIENT_KEY_EXCHANGE {
                            let il = usize::from(u16::from_be_bytes([m.body[0], m.body[1]]));
                            if m.body[2..2 + il] != o.identity[..] {
                                return None;
                            }
                            transcript.extend_from_slice(&hs_message(
                                HS_CLIENT_KEY_EXCHANGE,
                                m.seq,
                                &m.body,
                            ));
                            got_cke = true;
                        }
                    }
                }
                (CT_HANDSHAKE, 1) if got_cke => {
                    // Without the right key this record is noise to us, as a real server treats it: no answer.
                    let Some(plain) = keys.client.open(h.ct, h.epoch, h.seq, f) else {
                        continue;
                    };
                    let ms = reasm.push(&plain);
                    let Some(fin) = ms.iter().find(|m| m.msg_type == HS_FINISHED) else {
                        continue;
                    };
                    if ct_eq(
                        &fin.body,
                        &verify_data(&master, b"client finished", &transcript),
                    ) {
                        transcript.extend_from_slice(&hs_message(HS_FINISHED, fin.seq, &fin.body));
                    }
                }
                _ => {}
            }
        }
        if reasm_has_finished(&transcript) {
            break;
        }
    }

    // Our ChangeCipherSpec and Finished.
    let fin = verify_data(&master, b"server finished", &transcript);
    let fin_msg = hs_message(HS_FINISHED, msg_seq, &fin);
    let ccs = record(CT_CCS, 0, seq0, &[1]);
    let enc = record(
        CT_HANDSHAKE,
        1,
        seq1,
        &keys.server.seal(CT_HANDSHAKE, 1, seq1, &fin_msg),
    );
    seq1 += 1;
    if o.finished_before_ccs {
        sock.send_to(&enc, peer).ok()?;
        thread::sleep(Duration::from_millis(50));
        sock.send_to(&ccs, peer).ok()?;
    } else {
        sock.send_to(&[ccs, enc].concat(), peer).ok()?;
    }

    // Application data until the client closes (or we do).
    let closer = o.close_after.map(|d| Instant::now() + d);
    loop {
        if let Some(t) = closer {
            if Instant::now() >= t {
                let frag = keys.server.seal(CT_ALERT, 1, seq1, &[1, 0]);
                sock.send_to(&record(CT_ALERT, 1, seq1, &frag), peer).ok()?;
                return Some(());
            }
            sock.set_read_timeout(Some(Duration::from_millis(20)))
                .ok()?;
        }
        let Ok((n, _)) = sock.recv_from(&mut buf) else {
            if closer.is_some() {
                continue;
            }
            return None;
        };
        for (h, f) in split_records(&buf[..n]) {
            if h.epoch != 1 {
                continue;
            }
            let Some(plain) = keys.client.open(h.ct, h.epoch, h.seq, f) else {
                continue;
            };
            match h.ct {
                CT_APP => {
                    let _ = tx.send(plain);
                }
                CT_ALERT if plain == [1, 0] => {
                    let _ = tx.send(b"<close_notify>".to_vec());
                    return Some(());
                }
                _ => {}
            }
        }
    }
}

/// The transcript only ends with the client's Finished once the epoch-1 record was verified and appended; the
/// ClientKeyExchange alone does not mean the flight is complete.
fn reasm_has_finished(transcript: &[u8]) -> bool {
    // Messages are appended whole, so the last header's type tells what came last.
    let mut p = 0;
    let mut last = 0;
    while p + HS_HEADER <= transcript.len() {
        let len = (usize::from(transcript[p + 1]) << 16)
            | (usize::from(transcript[p + 2]) << 8)
            | usize::from(transcript[p + 3]);
        last = transcript[p];
        p += HS_HEADER + len;
    }
    last == HS_FINISHED
}
