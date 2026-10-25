//! WLED's UDP realtime input over DDP (Distributed Display Protocol, UDP port 4048).
//!
//! A packet is a 10-byte header and up to 1440 bytes of pixel data (480 RGB LEDs):
//!
//! ```text
//!   byte 0     flags: 0x40 version 1, 0x01 push (show now); the last packet of a frame carries push
//!   byte 1     sequence 1..15 (0 = unsequenced)
//!   byte 2     data type: 0x0B RGB 8 bit, 0x1B RGBW 8 bit
//!   byte 3     destination id: 1 = the default output device
//!   bytes 4-7  offset into the device's pixel data in bytes, big endian
//!   bytes 8-9  length of the data that follows, big endian
//! ```
//!
//! A longer strip is several packets with growing offsets and push on the last. WLED leaves realtime mode about
//! 2.5 s after the last packet (its `realtime timeout`), so [`Sender::send`] is called at the output rate even when
//! the picture is still. ESPHome's WLED effect and any DDP receiver take the same packets.

use std::io;
use std::net::{SocketAddr, UdpSocket};

pub const PORT: u16 = 4048;
pub const MAX_DATA: usize = 1440;
pub const HEADER: usize = 10;
const VER1: u8 = 0x40;
const PUSH: u8 = 0x01;
const DEFAULT_ID: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgb,
    Rgbw,
}

impl Format {
    fn type_byte(self) -> u8 {
        match self {
            Format::Rgb => 0x0B,
            Format::Rgbw => 0x1B,
        }
    }

    pub fn bytes_per_led(self) -> usize {
        match self {
            Format::Rgb => 3,
            Format::Rgbw => 4,
        }
    }
}

/// One packet. `seq` is masked to a nibble.
pub fn encode(seq: u8, format: Format, offset: u32, data: &[u8], push: bool) -> Vec<u8> {
    assert!(data.len() <= MAX_DATA, "a DDP packet carries at most {MAX_DATA} bytes");
    let mut p = Vec::with_capacity(HEADER + data.len());
    p.push(VER1 | if push { PUSH } else { 0 });
    p.push(seq & 0x0F);
    p.push(format.type_byte());
    p.push(DEFAULT_ID);
    p.extend_from_slice(&offset.to_be_bytes());
    p.extend_from_slice(&(data.len() as u16).to_be_bytes());
    p.extend_from_slice(data);
    p
}

/// The next sequence number: 1..=15, never 0.
pub fn next_seq(seq: u8) -> u8 {
    if seq >= 15 || seq == 0 { 1 } else { seq + 1 }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub push: bool,
    pub seq: u8,
    pub data_type: u8,
    pub id: u8,
    pub offset: u32,
    pub data: &'a [u8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Short,
    Version(u8),
    Length { declared: usize, available: usize },
}

/// Parses a packet without the optional timecode (the only form this crate sends).
pub fn parse(p: &[u8]) -> Result<Packet<'_>, ParseError> {
    if p.len() < HEADER {
        return Err(ParseError::Short);
    }
    if p[0] & 0xC0 != VER1 {
        return Err(ParseError::Version(p[0] >> 6));
    }
    let declared = usize::from(u16::from_be_bytes([p[8], p[9]]));
    let available = p.len() - HEADER;
    if declared > available {
        return Err(ParseError::Length { declared, available });
    }
    Ok(Packet {
        push: p[0] & PUSH != 0,
        seq: p[1] & 0x0F,
        data_type: p[2],
        id: p[3],
        offset: u32::from_be_bytes([p[4], p[5], p[6], p[7]]),
        data: &p[HEADER..HEADER + declared],
    })
}

/// Sends frames of pixel data to one DDP receiver.
pub struct Sender {
    socket: UdpSocket,
    seq: u8,
    format: Format,
}

impl Sender {
    pub fn connect(addr: SocketAddr, format: Format) -> io::Result<Sender> {
        let bind: SocketAddr = if addr.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
        let socket = UdpSocket::bind(bind)?;
        socket.connect(addr)?;
        Ok(Sender { socket, seq: 0, format })
    }

    /// Sends one frame (`pixels` is RGB or RGBW bytes for the whole strip) as as many packets as it needs;
    /// returns how many. An empty frame sends nothing.
    pub fn send(&mut self, pixels: &[u8]) -> io::Result<usize> {
        // Chunks hold whole LEDs so a receiver never sees a pixel split across packets.
        let per = self.format.bytes_per_led();
        let chunk = MAX_DATA / per * per;
        let mut sent = 0;
        let mut offset = 0usize;
        while offset < pixels.len() {
            let end = (offset + chunk).min(pixels.len());
            self.seq = next_seq(self.seq);
            let pkt = encode(self.seq, self.format, offset as u32, &pixels[offset..end], end == pixels.len());
            self.socket.send(&pkt)?;
            sent += 1;
            offset = end;
        }
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn header_bytes_match_the_protocol() {
        let p = encode(3, Format::Rgb, 0x0102_0304, &[9, 8, 7], true);
        assert_eq!(p, vec![0x41, 0x03, 0x0B, 0x01, 0x01, 0x02, 0x03, 0x04, 0x00, 0x03, 9, 8, 7]);
        let p = encode(0x1F, Format::Rgbw, 0, &[], false);
        assert_eq!(p, vec![0x40, 0x0F, 0x1B, 0x01, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn parse_round_trips_and_rejects_bad_packets() {
        let raw = encode(7, Format::Rgb, 90, &[1, 2, 3, 4, 5, 6], false);
        let p = parse(&raw).unwrap();
        assert_eq!((p.push, p.seq, p.data_type, p.id, p.offset, p.data), (false, 7, 0x0B, 1, 90, &[1u8, 2, 3, 4, 5, 6][..]));
        assert_eq!(parse(&raw[..9]), Err(ParseError::Short));
        let mut v2 = raw.clone();
        v2[0] = 0x80;
        assert_eq!(parse(&v2), Err(ParseError::Version(2)));
        assert_eq!(parse(&raw[..raw.len() - 1]), Err(ParseError::Length { declared: 6, available: 5 }));
    }

    #[test]
    fn sequence_cycles_one_to_fifteen() {
        let mut s = 0;
        let seen: Vec<u8> = (0..32)
            .map(|_| {
                s = next_seq(s);
                s
            })
            .collect();
        assert_eq!(&seen[..16], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 1]);
        assert!(seen.iter().all(|&v| (1..=15).contains(&v)));
    }

    /// A stand-in WLED: a UDP socket that rebuilds the strip from DDP packets the way a receiver does.
    struct FakeWled {
        socket: UdpSocket,
    }

    impl FakeWled {
        fn new() -> FakeWled {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            FakeWled { socket }
        }

        fn addr(&self) -> SocketAddr {
            self.socket.local_addr().unwrap()
        }

        /// Reads packets until one with push; returns (strip bytes, packets, seqs, push flags).
        fn frame(&self, len: usize) -> (Vec<u8>, usize, Vec<u8>, Vec<bool>) {
            let mut strip = vec![0u8; len];
            let mut buf = [0u8; 2048];
            let (mut n, mut seqs, mut pushes) = (0, Vec::new(), Vec::new());
            loop {
                let got = self.socket.recv(&mut buf).expect("a DDP packet");
                let p = parse(&buf[..got]).expect("valid DDP");
                assert!(p.data.len() <= MAX_DATA);
                let o = p.offset as usize;
                strip[o..o + p.data.len()].copy_from_slice(p.data);
                n += 1;
                seqs.push(p.seq);
                pushes.push(p.push);
                if p.push {
                    return (strip, n, seqs, pushes);
                }
            }
        }
    }

    #[test]
    fn a_short_strip_is_one_packet_with_push() {
        let wled = FakeWled::new();
        let mut s = Sender::connect(wled.addr(), Format::Rgb).unwrap();
        let pixels: Vec<u8> = (0..30).collect();
        assert_eq!(s.send(&pixels).unwrap(), 1);
        let (strip, n, seqs, pushes) = wled.frame(30);
        assert_eq!((strip, n, seqs, pushes), (pixels, 1, vec![1], vec![true]));
    }

    #[test]
    fn a_long_strip_is_split_on_whole_leds_with_push_on_the_last() {
        let wled = FakeWled::new();
        let mut s = Sender::connect(wled.addr(), Format::Rgb).unwrap();
        // 700 LEDs = 2100 bytes: 1440 + 660.
        let pixels: Vec<u8> = (0..2100u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(s.send(&pixels).unwrap(), 2);
        let (strip, n, seqs, pushes) = wled.frame(2100);
        assert_eq!(strip, pixels);
        assert_eq!((n, seqs, pushes), (2, vec![1, 2], vec![false, true]));
    }

    #[test]
    fn rgbw_chunks_hold_whole_four_byte_leds() {
        let wled = FakeWled::new();
        let mut s = Sender::connect(wled.addr(), Format::Rgbw).unwrap();
        // 1440 is divisible by 4, but 1441 bytes of 4-byte LEDs must not split mid LED: 400 LEDs = 1600 bytes.
        let pixels: Vec<u8> = (0..1600u32).map(|i| (i % 239) as u8).collect();
        s.send(&pixels).unwrap();
        let (strip, n, ..) = wled.frame(1600);
        assert_eq!((strip, n), (pixels, 2));
    }

    #[test]
    fn sequence_continues_across_frames_and_wraps() {
        let wled = FakeWled::new();
        let mut s = Sender::connect(wled.addr(), Format::Rgb).unwrap();
        let mut all = Vec::new();
        for i in 0..17u8 {
            s.send(&[i, i, i]).unwrap();
            all.push(wled.frame(3).2[0]);
        }
        assert_eq!(&all[13..], &[14, 15, 1, 2]);
    }

    #[test]
    fn an_empty_frame_sends_nothing() {
        let wled = FakeWled::new();
        let mut s = Sender::connect(wled.addr(), Format::Rgb).unwrap();
        assert_eq!(s.send(&[]).unwrap(), 0);
        wled.socket.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        assert!(wled.socket.recv(&mut [0u8; 64]).is_err());
    }
}
