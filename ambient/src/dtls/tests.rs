//! Known-answer tests for the primitives (vectors computed with Python's hmac, hashlib and cryptography), and the
//! handshake against an in-process server that plays the server side of RFC 6347 with the same primitives. The
//! interop test against OpenSSL's `s_server` (tests/openssl_interop.rs) is what shows the two sides agree with a
//! second implementation rather than with each other.

use super::*;
use crate::dtls::testserver::{PSK, ServerOpts, TestServer, fragment, hex, spawn_server};
use std::thread;

fn arr<const N: usize>(v: Vec<u8>) -> [u8; N] {
    v.try_into().unwrap()
}

#[test]
fn prf_matches_the_published_tls12_sha256_vector() {
    let mut out = [0u8; 100];
    prf(
        &hex("9bbe436ba940f017b17652849a71db35"),
        b"test label",
        &hex("a0ba9f936cda311827a6f796ffd5198c"),
        &mut out,
    );
    assert_eq!(
        out.to_vec(),
        hex(
            "e3f229ba727be17b8d122620557cd453c2aab21d07c3d495329b52d4e61edb5a6b301791e90d35c9c9a46b4e14baf9af0fa022f7077def17abfd3797c0564bab4fbc91666e9def9b97fce34f796789baa48082d122ee42c5a72e5a5110fff70187347b66"
        )
    );
}

fn randoms() -> ([u8; 32], [u8; 32]) {
    (arr((0..32).collect()), arr((32..64).collect()))
}

#[test]
fn psk_premaster_master_keys_and_finished_match_the_reference() {
    let psk = hex(PSK);
    assert_eq!(
        psk_premaster(&psk),
        hex("001000000000000000000000000000000000001000112233445566778899aabbccddeeff")
    );
    let (cr, sr) = randoms();
    let master = master_secret(&psk, &cr, &sr);
    assert_eq!(
        master.to_vec(),
        hex(
            "5c2a8605397d6f20193cb3a89f51e450845b98164b4578c31aa6a27588b3d8e4a91318a995788133338dfc8bbe2daa94"
        )
    );
    // The key block is 280468ab... : client key, server key, client salt, server salt. Check it through the ciphers.
    let keys = derive_keys(&master, &cr, &sr);
    let kb =
        hex("280468abb22e9359d62c64ef76da06a244c41001da6e33e8c071cb9691ba336694a06300d009c85e");
    assert_eq!(keys.client.salt.to_vec(), kb[32..36]);
    assert_eq!(keys.server.salt.to_vec(), kb[36..40]);
    let by_hand = Gcm::new(&kb[0..16], &kb[32..36]);
    assert_eq!(
        keys.client.seal(CT_APP, 1, 3, b"x"),
        by_hand.seal(CT_APP, 1, 3, b"x")
    );
    let server_by_hand = Gcm::new(&kb[16..32], &kb[36..40]);
    assert_eq!(
        keys.server.seal(CT_APP, 1, 3, b"x"),
        server_by_hand.seal(CT_APP, 1, 3, b"x")
    );
    assert_ne!(
        keys.client.seal(CT_APP, 1, 3, b"x"),
        keys.server.seal(CT_APP, 1, 3, b"x")
    );
    assert_eq!(
        verify_data(&master, b"client finished", b"abc").to_vec(),
        hex("525ab2d1c9f4f30ccde2a2c8")
    );
    assert_eq!(
        verify_data(&master, b"server finished", b"abc").to_vec(),
        hex("81215317b54b9c10839aed5d")
    );
}

#[test]
fn gcm_record_matches_the_reference_and_rejects_tampering() {
    let g = Gcm::new(&(0..16).collect::<Vec<u8>>(), &[0xa0, 0xa1, 0xa2, 0xa3]);
    let sealed = g.seal(CT_APP, 1, 5, b"HueStream-test");
    assert_eq!(
        sealed,
        hex("0001000000000005847f34d550e58ff59968c79a0769765b620df03b37abeb2f82a27d232636")
    );
    assert_eq!(
        g.open(CT_APP, 1, 5, &sealed).as_deref(),
        Some(&b"HueStream-test"[..])
    );
    // Wrong sequence number, wrong type, flipped bit, truncated: all refused.
    assert!(g.open(CT_APP, 1, 6, &sealed).is_none());
    assert!(g.open(CT_HANDSHAKE, 1, 5, &sealed).is_none());
    let mut bad = sealed.clone();
    bad[10] ^= 1;
    assert!(g.open(CT_APP, 1, 5, &bad).is_none());
    assert!(g.open(CT_APP, 1, 5, &sealed[..20]).is_none());
    assert!(g.open(CT_APP, 1, 5, &[]).is_none());
}

#[test]
fn records_split_out_of_one_datagram() {
    let d = [
        record(CT_HANDSHAKE, 0, 7, b"abc"),
        record(CT_CCS, 0, 8, &[1]),
        record(CT_APP, 1, 0x0102_0304_0506, b"zz"),
    ]
    .concat();
    let r = split_records(&d);
    assert_eq!(r.len(), 3);
    assert_eq!(
        (r[0].0.ct, r[0].0.epoch, r[0].0.seq, r[0].1),
        (CT_HANDSHAKE, 0, 7, &b"abc"[..])
    );
    assert_eq!((r[2].0.epoch, r[2].0.seq), (1, 0x0102_0304_0506));
    // A record that claims more than the datagram holds ends the walk; so does a non-DTLS version.
    assert_eq!(split_records(&d[..d.len() - 1]).len(), 2);
    let mut v = d.clone();
    v[1] = 3;
    assert!(split_records(&v).is_empty());
}

#[test]
fn reassembly_joins_fragments_in_any_order_and_releases_in_sequence() {
    let m0: Vec<u8> = (0..100).collect();
    let m1: Vec<u8> = (100..110).collect();
    let mut r = Reassembler::default();
    // Message 1 complete first: held back, because message 0 is not there yet.
    assert!(r.push(&fragment(2, 1, &m1, 0, 10)).is_empty());
    // Message 0 in three fragments, last one first.
    assert!(r.push(&fragment(2, 0, &m0, 60, 40)).is_empty());
    assert!(r.push(&fragment(2, 0, &m0, 0, 30)).is_empty());
    let out = r.push(&fragment(2, 0, &m0, 30, 30));
    assert_eq!(out.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(out[0].body, m0);
    assert_eq!(out[1].body, m1);
    assert_eq!(r.next_seq(), 2);
    // A retransmission of a message already released is dropped.
    assert!(r.push(&fragment(2, 0, &m0, 0, 100)).is_empty());
    // Two messages in one record payload.
    let mut two = fragment(14, 2, &[], 0, 0);
    two.extend(fragment(20, 3, &[1, 2, 3], 0, 3));
    let out = r.push(&two);
    assert_eq!(
        out.iter().map(|m| (m.msg_type, m.seq)).collect::<Vec<_>>(),
        vec![(14, 2), (20, 3)]
    );
}

fn cfg(o: &ServerOpts) -> PskConfig {
    let mut c = PskConfig::new(&o.identity, &o.psk);
    c.first_retransmit = Duration::from_millis(150);
    c.handshake_timeout = Duration::from_secs(4);
    c
}

fn recv_app(s: &TestServer) -> Vec<u8> {
    s.rx.recv_timeout(Duration::from_secs(3))
        .expect("application data at the server")
}

#[test]
fn handshake_then_application_data_in_order() {
    let o = ServerOpts::new();
    let s = spawn_server(o.clone());
    let mut c = PskClient::connect(s.addr, &cfg(&o)).expect("handshake");
    for i in 0..5u8 {
        c.send(&[b'H', i, 0, 0xFF]).unwrap();
    }
    for i in 0..5u8 {
        assert_eq!(recv_app(&s), vec![b'H', i, 0, 0xFF]);
    }
    c.close();
    assert_eq!(recv_app(&s), b"<close_notify>");
}

#[test]
fn hello_verify_request_round_is_handled() {
    let o = ServerOpts {
        hello_verify: true,
        ..ServerOpts::new()
    };
    let s = spawn_server(o.clone());
    let mut c = PskClient::connect(s.addr, &cfg(&o)).expect("handshake with cookie");
    c.send(b"after-cookie").unwrap();
    assert_eq!(recv_app(&s), b"after-cookie");
}

#[test]
fn fragmented_server_hello_and_key_exchange_hint() {
    let o = ServerOpts {
        fragment_server_hello: true,
        key_exchange_hint: true,
        ..ServerOpts::new()
    };
    let s = spawn_server(o.clone());
    let mut c = PskClient::connect(s.addr, &cfg(&o)).expect("handshake");
    c.send(b"frag").unwrap();
    assert_eq!(recv_app(&s), b"frag");
}

#[test]
fn a_lost_client_flight_is_resent_after_the_timer() {
    let o = ServerOpts {
        drop_first_client_flight: true,
        ..ServerOpts::new()
    };
    let s = spawn_server(o.clone());
    let t = Instant::now();
    let mut c = PskClient::connect(s.addr, &cfg(&o)).expect("handshake after one loss");
    assert!(
        t.elapsed() >= Duration::from_millis(140),
        "waited for the timer: {:?}",
        t.elapsed()
    );
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert!(
        s.seen_flights.try_iter().count() >= 2,
        "the server saw the flight twice"
    );
    c.send(b"survived").unwrap();
    assert_eq!(recv_app(&s), b"survived");
}

#[test]
fn an_encrypted_finished_that_overtakes_the_ccs_is_held_until_it_arrives() {
    let o = ServerOpts {
        finished_before_ccs: true,
        ..ServerOpts::new()
    };
    let s = spawn_server(o.clone());
    let mut c = PskClient::connect(s.addr, &cfg(&o)).expect("handshake with reordering");
    c.send(b"reordered").unwrap();
    assert_eq!(recv_app(&s), b"reordered");
}

#[test]
fn wrong_key_ends_in_a_timeout_not_a_hang() {
    let server = ServerOpts::new();
    let s = spawn_server(server.clone());
    let mut c = cfg(&server);
    c.psk = hex("ffeeddccbbaa99887766554433221100");
    c.handshake_timeout = Duration::from_millis(700);
    let t = Instant::now();
    match PskClient::connect(s.addr, &c) {
        Err(DtlsError::Timeout) => {}
        other => panic!("expected a timeout, got {:?}", other.map(|_| ())),
    }
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn nobody_listening_ends_in_a_timeout() {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    let o = ServerOpts::new();
    let mut c = cfg(&o);
    c.handshake_timeout = Duration::from_millis(500);
    drop(sock);
    assert!(matches!(
        PskClient::connect(addr, &c),
        Err(DtlsError::Timeout | DtlsError::Io(_))
    ));
}

#[test]
fn a_close_notify_from_the_server_shows_up_in_poll() {
    let o = ServerOpts {
        close_after: Some(Duration::from_millis(200)),
        ..ServerOpts::new()
    };
    let s = spawn_server(o.clone());
    let mut c = PskClient::connect(s.addr, &cfg(&o)).expect("handshake");
    assert!(c.poll().is_ok(), "nothing yet");
    thread::sleep(Duration::from_millis(500));
    assert!(matches!(c.poll(), Err(DtlsError::Closed)));
    drop(s);
}

#[test]
fn a_fatal_alert_during_the_handshake_is_reported() {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        if let Ok((_, from)) = sock.recv_from(&mut buf) {
            let _ = sock.send_to(&record(CT_ALERT, 0, 0, &[2, 40]), from); // fatal handshake_failure
        }
    });
    let o = ServerOpts::new();
    match PskClient::connect(addr, &cfg(&o)) {
        Err(DtlsError::Alert {
            level: 2,
            description: 40,
        }) => {}
        other => panic!("{:?}", other.map(|_| ())),
    }
}
