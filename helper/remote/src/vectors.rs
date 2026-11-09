//! The protocol as exact bytes, for LumaRemoteKit's tests and any other client. `lumaremote vectors` writes them; `tests/vectors.json` is
//! checked against [`generate`]. Every value comes from the engine's own code. Bytes are lowercase hex.

use esphome_api::base64;
use esphome_api::noise::{Initiator, Responder};
use serde_json::{Value, json};

use crate::datagram::{self, Accumulator, Button, Event, Header, Key, Pointer};
use crate::hex;
use crate::pairing;
use crate::wire::{self, Mode};

/// The TV id of the first pairing vector and of the frames.
pub const TV: [u8; 8] = [0x0a, 1, 2, 3, 4, 5, 6, 0xff];
/// Typed codes a phone refuses before connecting: too short, too long, characters outside the alphabet.
pub const REFUSED: [&str; 6] = ["K7WDX2A", "K7WDX2ABC", "K7WDX2A1", "K7WDX2AO", "K7WDX2AI", "K7WD_X2AB"];
/// The UDP key and session of the datagram vectors and of the session vector's `welcome`.
pub const UDP_KEY: [u8; 32] = [0x42; 32];
pub const UDP_SESSION: u32 = 0x0102_0304;

/// `start`, `start + 1`, …
fn counting(start: u8) -> [u8; 32] {
    std::array::from_fn(|i| start + i as u8)
}

pub fn generate() -> Value {
    json!({
        "version": crate::VERSION,
        "pairing": pairing_keys(),
        "refused": REFUSED,
        "handshakes": [session(), pair()],
        "datagrams": datagrams(),
        "pointer": pointer_sequences(),
    })
}

/// Pairing keys: a typed code, the code it normalizes to, as the TV shows it, the TV id and the key.
fn pairing_keys() -> Vec<Value> {
    [("k7wd-x2ab", TV), ("2345 6789", [0; 8]), ("ZZZZ-ZZZZ", [0xff; 8])]
        .iter()
        .map(|(typed, tv)| {
            let code = pairing::normalize(typed).expect("a valid code");
            json!({"typed": typed, "code": code, "display": pairing::display(&code), "tv": hex(tv), "key": hex(&pairing::key(&code, tv))})
        })
        .collect()
}

/// A handshake with fixed ephemeral secrets (the phone's bytes count up from 0x40, the TV's from 0x80), then frames each way in order: `json`
/// is a frame's exact plaintext, `frame` its length and ciphertext.
fn handshake(mode: Mode, psk: &[u8; 32], frames: &[(&str, String)]) -> Value {
    let preamble = mode.preamble();
    let (phone_secret, tv_secret) = (counting(0x40), counting(0x80));
    let (initiator, first) = Initiator::start_with(&preamble, psk, phone_secret);
    let (second, mut tv) = Responder::respond_with(&preamble, psk, tv_secret, &first).expect("the same key");
    let mut phone = initiator.finish(&second).expect("the same key");
    let frames: Vec<Value> = frames
        .iter()
        .map(|(from, json)| {
            let cipher = if *from == "phone" { &mut phone.send } else { &mut tv.send };
            json!({"from": from, "json": json, "frame": hex(&wire::seal(cipher, json.as_bytes()).expect("a short frame"))})
        })
        .collect();
    json!({
        "mode": match mode { Mode::Session => "session", Mode::Pair => "pair" },
        "preamble": hex(&preamble),
        "psk": hex(psk),
        "phone_ephemeral": hex(&phone_secret),
        "tv_ephemeral": hex(&tv_secret),
        "message1": hex(&first),
        "message2": hex(&second),
        "frames": frames,
    })
}

/// The session of the phone [`pair`] paired: its key counts up from 0x20.
fn session() -> Value {
    let welcome = format!(
        r##"{{"t":"welcome","v":1,"device":"0123456789abcdef","tv":{{"id":"{}","name":"LG C4","mac":"74:c1:7e:bd:ae:30","accent":"#5b8cff"}},"udp":{{"session":{UDP_SESSION},"key":"{}"}},"state":{{"power":"on","fg":"","ime":{{"active":false}},"volume":{{"via":"receiver","level":40,"max":98,"muted":false}},"acts":["home","quick","input","launch","volume","power"]}}}}"##,
        hex(&TV),
        base64::encode(&UDP_KEY),
    );
    let frames = [
        ("phone", r#"{"t":"hello","name":"Test phone","model":"iPhone17,1","app":"LumaRemote 1.0","kit":"1.0"}"#.to_string()),
        ("tv", welcome),
        ("phone", r#"{"t":"key","k":"ok","a":"tap"}"#.to_string()),
        ("tv", r#"{"t":"state","fg":"com.beb.plxnative"}"#.to_string()),
    ];
    handshake(Mode::Session, &counting(0x20), &frames)
}

/// Pairing with the first pairing vector's code.
fn pair() -> Value {
    let paired = format!(
        r#"{{"t":"paired","device":"0123456789abcdef","key":"{}","tv":{{"id":"{}","name":"LG C4","mac":"74:c1:7e:bd:ae:30"}}}}"#,
        base64::encode(&counting(0x20)),
        hex(&TV),
    );
    let frames = [("phone", r#"{"t":"pair","name":"Test phone","model":"iPhone17,1","app":"LumaRemote 1.0"}"#.to_string()), ("tv", paired)];
    handshake(Mode::Pair, &pairing::key("K7WDX2AB", &TV), &frames)
}

/// One datagram of each kind: header, nonce, plaintext and the packet sent.
fn datagrams() -> Value {
    let key = Key::new(&UDP_KEY);
    let pointer = Pointer { x: 32, y: -16, buttons: datagram::LEFT, left: 1, flags: datagram::ENGAGED, ..Pointer::default() };
    let items = [
        (datagram::POINTER, datagram::TO_TV, 1, pointer.encode().to_vec()),
        (datagram::PING, datagram::TO_TV, 2, datagram::ping(7, 1_234_567_890).to_vec()),
        (datagram::PONG, datagram::TO_PHONE, 1, datagram::pong(7, 1_234_567_890, 250).to_vec()),
    ];
    let items: Vec<Value> = items
        .iter()
        .map(|(kind, direction, seq, plain)| {
            let header = Header { kind: *kind, session: UDP_SESSION, seq: *seq };
            json!({
                "kind": kind, "direction": direction, "seq": seq,
                "header": hex(&header.encode()), "nonce": hex(&datagram::nonce(*direction, *seq)),
                "plain": hex(plain), "packet": hex(&key.seal(*direction, header, plain)),
            })
        })
        .collect();
    json!({"key": hex(&UDP_KEY), "session": UDP_SESSION, "items": items})
}

fn totals(p: &Pointer) -> Value {
    json!({
        "x": p.x, "y": p.y, "scroll_x": p.scroll_x, "scroll_y": p.scroll_y,
        "buttons": p.buttons, "left": p.left, "right": p.right, "centre": p.centre, "flags": p.flags,
    })
}

fn event(e: &Event) -> Value {
    let button = |b: &Button| match b {
        Button::Left => "left",
        Button::Right => "right",
    };
    match e {
        Event::Engaged(on) => json!({"e": "engaged", "on": on}),
        Event::Centre => json!({"e": "centre"}),
        Event::Move { dx, dy } => json!({"e": "move", "dx": dx, "dy": dy}),
        Event::Scroll { dx, dy } => json!({"e": "scroll", "dx": dx, "dy": dy}),
        Event::Click(b) => json!({"e": "click", "button": button(b)}),
        Event::Press(b) => json!({"e": "press", "button": button(b)}),
        Event::Release(b) => json!({"e": "release", "button": button(b)}),
    }
}

/// The totals the TV last accepted, then each datagram's totals, plaintext and what the TV makes of it. A `lost` one never arrives: the next
/// one carries what it held.
fn sequence(name: &str, start: Pointer, steps: &[(Pointer, bool)]) -> Value {
    let mut tv = Accumulator::default();
    tv.apply(&start, &mut Vec::new());
    let steps: Vec<Value> = steps
        .iter()
        .map(|(p, lost)| {
            let mut step = json!({"totals": totals(p), "plain": hex(&p.encode())});
            if *lost {
                step["lost"] = true.into();
            } else {
                let mut events = Vec::new();
                tv.apply(p, &mut events);
                step["events"] = events.iter().map(event).collect();
            }
            step
        })
        .collect();
    json!({"name": name, "start": totals(&start), "steps": steps})
}

fn pointer_sequences() -> Vec<Value> {
    use datagram::{ENGAGED as E, LEFT as L, RIGHT as R, SCROLLING as S};
    let at = |x, y, buttons, left, centre, flags| Pointer { x, y, buttons, left, centre, flags, ..Pointer::default() };
    let scroll = |scroll_x, scroll_y, buttons, right, flags| Pointer { scroll_x, scroll_y, buttons, right, flags, ..Pointer::default() };
    let wrap = Pointer { x: i32::MAX - 600, y: i32::MIN + 600, scroll_y: i32::MAX, left: 254, centre: 255, flags: E, ..Pointer::default() };
    let wrapped = Pointer { x: wrap.x.wrapping_add(1600), y: wrap.y.wrapping_sub(1600), scroll_y: wrap.scroll_y.wrapping_add(64), left: 0, centre: 0, ..wrap };
    vec![
        sequence(
            "click-drag",
            Pointer::default(),
            &[
                (at(0, 0, 0, 0, 0, E), false),
                (at(160, 80, 0, 0, 0, E), false),
                (at(160, 80, L, 1, 0, E), false),
                (at(480, 80, L, 1, 0, E), false),
                (at(480, 80, 0, 1, 0, E), false),
                (at(480, 80, 0, 2, 0, E), false),
                (at(480, 80, 0, 2, 1, E), false),
                (at(480, 80, 0, 2, 1, 0), false),
            ],
        ),
        sequence(
            "scroll-right-button",
            Pointer::default(),
            &[
                (scroll(0, 0, 0, 0, E | S), false),
                (scroll(0, 320, 0, 0, E | S), false),
                (scroll(-48, 320, 0, 0, E | S), false),
                (scroll(-48, 320, 0, 0, E), false),
                (scroll(-48, 320, 0, 1, E), false),
                (scroll(-48, 320, R, 2, E), false),
                (scroll(-48, 320, 0, 2, E), false),
            ],
        ),
        sequence("wrap", wrap, &[(wrapped, false)]),
        sequence(
            "lost",
            Pointer::default(),
            &[
                (at(64, 0, 0, 0, 0, E), false),
                (at(128, 0, 0, 1, 0, E), true),
                (at(192, 0, L, 2, 0, E), false),
                (at(192, 0, 0, 2, 0, E), true),
                (at(192, 0, 0, 2, 0, 0), false),
            ],
        ),
    ]
}
