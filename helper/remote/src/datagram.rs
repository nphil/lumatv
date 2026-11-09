//! UDP datagrams: a 12-byte header in clear, authenticated as associated data, then a ChaCha20-Poly1305 body under the session's UDP key.
//! A pointer datagram carries totals since the session began rather than changes, so a lost datagram loses nothing: the next one's totals
//! include it. [`Accumulator`] turns totals back into what happened.

use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};

pub const HEADER_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const POINTER_LEN: usize = 24;
pub const PING_LEN: usize = 12;
pub const PONG_LEN: usize = 16;

/// Kinds.
pub const POINTER: u8 = 1;
pub const PING: u8 = 2;
pub const PONG: u8 = 3;

/// The direction byte of the nonce.
pub const TO_TV: u8 = 0;
pub const TO_PHONE: u8 = 1;

/// Pointer `buttons` bits.
pub const LEFT: u8 = 1;
pub const RIGHT: u8 = 2;
/// Pointer `flags` bits: engaged (show the pointer and keep it shown), scrolling (fingers on the scroll surface).
pub const ENGAGED: u8 = 1;
pub const SCROLLING: u8 = 2;

/// The most motion one datagram applies, in 1/16 px (4,096 px).
pub const MAX_MOTION: i32 = 4096 * 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: u8,
    pub session: u32,
    /// From 1, up by one per datagram, per direction.
    pub seq: u32,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[..4].copy_from_slice(&[b'L', b'R', 1, self.kind]);
        b[4..8].copy_from_slice(&self.session.to_le_bytes());
        b[8..].copy_from_slice(&self.seq.to_le_bytes());
        b
    }

    pub fn parse(packet: &[u8]) -> Option<Header> {
        if packet.len() < HEADER_LEN + TAG_LEN || packet[..3] != [b'L', b'R', 1] {
            return None;
        }
        let word = |at: usize| u32::from_le_bytes(packet[at..at + 4].try_into().expect("4 bytes"));
        Some(Header { kind: packet[3], session: word(4), seq: word(8) })
    }
}

/// `4C 52`, the direction, a zero, then the sequence as a u64.
pub fn nonce(direction: u8, seq: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(&[0x4C, 0x52, direction, 0]);
    n[4..].copy_from_slice(&u64::from(seq).to_le_bytes());
    n
}

/// A session's UDP key.
pub struct Key(LessSafeKey);

impl Key {
    pub fn new(bytes: &[u8; 32]) -> Self {
        Key(LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, bytes).expect("32-byte key")))
    }

    /// The datagram: header, then the sealed body.
    pub fn seal(&self, direction: u8, header: Header, plain: &[u8]) -> Vec<u8> {
        let head = header.encode();
        let mut body = plain.to_vec();
        self.0
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce(direction, header.seq)), Aad::from(head), &mut body)
            .expect("a datagram is far below the AEAD's limit");
        let mut packet = head.to_vec();
        packet.extend_from_slice(&body);
        packet
    }

    /// The body of a datagram whose tag verifies.
    pub fn open(&self, direction: u8, packet: &[u8]) -> Option<Vec<u8>> {
        let header = Header::parse(packet)?;
        let mut body = packet[HEADER_LEN..].to_vec();
        let plain =
            self.0.open_in_place(Nonce::assume_unique_for_key(nonce(direction, header.seq)), Aad::from(&packet[..HEADER_LEN]), &mut body).ok()?;
        Some(plain.to_vec())
    }
}

/// A pointer datagram's plaintext: totals since the session began, wrapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointer {
    /// Motion in 1/16 TV pixel, right and down positive.
    pub x: i32,
    pub y: i32,
    /// Scrolling in 1/16 px; positive scrolls right and down (the content moves left and up).
    pub scroll_x: i32,
    pub scroll_y: i32,
    /// [`LEFT`] and [`RIGHT`] held.
    pub buttons: u8,
    /// Presses of each button.
    pub left: u8,
    pub right: u8,
    /// Each step puts the pointer at the centre.
    pub centre: u8,
    /// [`ENGAGED`], [`SCROLLING`].
    pub flags: u8,
}

impl Pointer {
    pub fn encode(&self) -> [u8; POINTER_LEN] {
        let mut b = [0u8; POINTER_LEN];
        for (i, v) in [self.x, self.y, self.scroll_x, self.scroll_y].into_iter().enumerate() {
            b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[16..21].copy_from_slice(&[self.buttons, self.left, self.right, self.centre, self.flags]);
        b
    }

    pub fn parse(plain: &[u8]) -> Option<Pointer> {
        if plain.len() != POINTER_LEN {
            return None;
        }
        let word = |i: usize| i32::from_le_bytes(plain[4 * i..4 * i + 4].try_into().expect("4 bytes"));
        Some(Pointer {
            x: word(0),
            y: word(1),
            scroll_x: word(2),
            scroll_y: word(3),
            buttons: plain[16],
            left: plain[17],
            right: plain[18],
            centre: plain[19],
            flags: plain[20],
        })
    }
}

/// `id`, then the phone's clock in µs.
pub fn ping(id: u32, time: u64) -> [u8; PING_LEN] {
    let mut b = [0u8; PING_LEN];
    b[..4].copy_from_slice(&id.to_le_bytes());
    b[4..].copy_from_slice(&time.to_le_bytes());
    b
}

pub fn parse_ping(plain: &[u8]) -> Option<(u32, u64)> {
    (plain.len() == PING_LEN).then(|| (u32::from_le_bytes(plain[..4].try_into().unwrap()), u64::from_le_bytes(plain[4..].try_into().unwrap())))
}

/// The ping's `id` and time echoed, and how long the TV held it in µs.
pub fn pong(id: u32, time: u64, hold: u32) -> [u8; PONG_LEN] {
    let mut b = [0u8; PONG_LEN];
    b[..12].copy_from_slice(&ping(id, time));
    b[12..].copy_from_slice(&hold.to_le_bytes());
    b
}

pub fn parse_pong(plain: &[u8]) -> Option<(u32, u64, u32)> {
    let (id, time) = parse_ping(plain.get(..PING_LEN)?)?;
    (plain.len() == PONG_LEN).then(|| (id, time, u32::from_le_bytes(plain[12..].try_into().unwrap())))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// What one pointer datagram did, in the order it is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The phone started pointing (`true`, first) or stopped (`false`, last).
    Engaged(bool),
    Centre,
    /// 1/16 px, at most [`MAX_MOTION`] each way.
    Move { dx: i32, dy: i32 },
    /// 1/16 px.
    Scroll { dx: i32, dy: i32 },
    Click(Button),
    Press(Button),
    Release(Button),
}

/// The last accepted totals of a session, which start at zero.
#[derive(Default)]
pub struct Accumulator {
    last: Pointer,
}

impl Accumulator {
    pub fn engaged(&self) -> bool {
        self.last.flags & ENGAGED != 0
    }

    /// What `now` changed since the last accepted totals: centre steps, motion, scroll, then each button: k presses with the button up are
    /// k clicks; with it held, k − 1 clicks and a press; no presses and the button newly up, a release.
    pub fn apply(&mut self, now: &Pointer, out: &mut Vec<Event>) {
        let last = std::mem::replace(&mut self.last, *now);
        let engaged = now.flags & ENGAGED != 0;
        if engaged && last.flags & ENGAGED == 0 {
            out.push(Event::Engaged(true));
        }
        if now.centre != last.centre {
            out.push(Event::Centre);
        }
        let (dx, dy) = (now.x.wrapping_sub(last.x), now.y.wrapping_sub(last.y));
        if dx != 0 || dy != 0 {
            out.push(Event::Move { dx: dx.clamp(-MAX_MOTION, MAX_MOTION), dy: dy.clamp(-MAX_MOTION, MAX_MOTION) });
        }
        let (sx, sy) = (now.scroll_x.wrapping_sub(last.scroll_x), now.scroll_y.wrapping_sub(last.scroll_y));
        if sx != 0 || sy != 0 {
            out.push(Event::Scroll { dx: sx, dy: sy });
        }
        for (button, bit, presses, before) in [(Button::Left, LEFT, now.left, last.left), (Button::Right, RIGHT, now.right, last.right)] {
            let k = presses.wrapping_sub(before);
            let held = now.buttons & bit != 0;
            if k > 0 {
                let clicks = if held { k - 1 } else { k };
                out.extend(std::iter::repeat_n(Event::Click(button), usize::from(clicks)));
                if held {
                    out.push(Event::Press(button));
                }
            } else if !held && last.buttons & bit != 0 {
                out.push(Event::Release(button));
            }
        }
        if !engaged && last.flags & ENGAGED != 0 {
            out.push(Event::Engaged(false));
        }
    }

    /// The session ended: held buttons are released and an engaged pointer let go.
    pub fn end(&mut self, out: &mut Vec<Event>) {
        let last = std::mem::take(&mut self.last);
        for (button, bit) in [(Button::Left, LEFT), (Button::Right, RIGHT)] {
            if last.buttons & bit != 0 {
                out.push(Event::Release(button));
            }
        }
        if last.flags & ENGAGED != 0 {
            out.push(Event::Engaged(false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(acc: &mut Accumulator, p: Pointer) -> Vec<Event> {
        let mut out = Vec::new();
        acc.apply(&p, &mut out);
        out
    }

    #[test]
    fn datagrams_open_only_with_their_key_direction_and_header() {
        let key = Key::new(&[3; 32]);
        let header = Header { kind: POINTER, session: 77, seq: 1 };
        let packet = key.seal(TO_TV, header, &Pointer { x: 16, ..Default::default() }.encode());
        assert_eq!(packet.len(), HEADER_LEN + POINTER_LEN + TAG_LEN);
        assert_eq!(Header::parse(&packet), Some(header));
        assert_eq!(Pointer::parse(&key.open(TO_TV, &packet).unwrap()).unwrap().x, 16);
        assert!(key.open(TO_PHONE, &packet).is_none(), "the other direction's nonce");
        assert!(Key::new(&[4; 32]).open(TO_TV, &packet).is_none());
        let mut moved = packet.clone();
        moved[8] = 2;
        assert!(key.open(TO_TV, &moved).is_none(), "the header is authenticated");
        assert_eq!(parse_pong(&pong(5, 1_000_000, 42)), Some((5, 1_000_000, 42)));
    }

    #[test]
    fn totals_survive_lost_datagrams_and_wrap_around() {
        let mut acc = Accumulator::default();
        let p = Pointer { x: i32::MAX - 15, flags: ENGAGED, ..Default::default() };
        assert_eq!(events(&mut acc, p), [Event::Engaged(true), Event::Move { dx: MAX_MOTION, dy: 0 }], "clamped");
        let p = Pointer { x: i32::MIN + 16, ..p };
        assert_eq!(events(&mut acc, p), [Event::Move { dx: 32, dy: 0 }], "wrapping");
        // Two presses, one lost with its release: two clicks. Then a press held, and its release.
        let p = Pointer { left: 2, ..p };
        assert_eq!(events(&mut acc, p), [Event::Click(Button::Left), Event::Click(Button::Left)]);
        let p = Pointer { left: 3, buttons: LEFT, ..p };
        assert_eq!(events(&mut acc, p), [Event::Press(Button::Left)]);
        let p = Pointer { buttons: 0, centre: 1, scroll_y: -48, ..p };
        assert_eq!(events(&mut acc, p), [Event::Centre, Event::Scroll { dx: 0, dy: -48 }, Event::Release(Button::Left)]);
        let p = Pointer { right: 1, buttons: RIGHT, ..p };
        assert_eq!(events(&mut acc, p), [Event::Press(Button::Right)]);
        let mut out = Vec::new();
        acc.end(&mut out);
        assert_eq!(out, [Event::Release(Button::Right), Event::Engaged(false)]);
    }
}
