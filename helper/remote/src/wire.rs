//! The TCP connection: a 6-byte preamble, the Noise NNpsk0 handshake's two 48-byte messages raw, then frames: a u16 length and that many
//! bytes of ciphertext, whose plaintext is one JSON object of at most [`MAX_JSON`] bytes.

use std::io::{self, Read, Write};

use esphome_api::noise::{CipherState, TAG_LEN};

use crate::VERSION;

pub use esphome_api::noise::HANDSHAKE_LEN;

pub const MAGIC: [u8; 4] = *b"LRMT";
pub const PREAMBLE_LEN: usize = 6;
pub const MAX_JSON: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Session,
    Pair,
}

impl Mode {
    /// The preamble, which is also the handshake's prologue.
    pub fn preamble(self) -> [u8; PREAMBLE_LEN] {
        let mode = match self {
            Mode::Session => 1,
            Mode::Pair => 2,
        };
        [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], VERSION, mode]
    }

    /// The mode a preamble asks for; `None` for another protocol or version.
    pub fn parse(preamble: &[u8; PREAMBLE_LEN]) -> Option<Mode> {
        if preamble[..4] != MAGIC || preamble[4] != VERSION {
            return None;
        }
        match preamble[5] {
            1 => Some(Mode::Session),
            2 => Some(Mode::Pair),
            _ => None,
        }
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

/// One sealed frame, length first.
pub fn seal(cipher: &mut CipherState, json: &[u8]) -> io::Result<Vec<u8>> {
    if json.len() > MAX_JSON {
        return Err(invalid("frame too long"));
    }
    let sealed = cipher.encrypt(json).map_err(|e| invalid(&e.to_string()))?;
    let mut frame = Vec::with_capacity(2 + sealed.len());
    frame.extend_from_slice(&(sealed.len() as u16).to_le_bytes());
    frame.extend_from_slice(&sealed);
    Ok(frame)
}

pub fn write(stream: &mut impl Write, cipher: &mut CipherState, json: &[u8]) -> io::Result<()> {
    stream.write_all(&seal(cipher, json)?)
}

/// Reads frames off a stream whose reads may time out in the middle of one: what has arrived is kept for the next call.
#[derive(Default)]
pub struct Reader {
    buf: Vec<u8>,
}

impl Reader {
    /// The next frame's plaintext. A length outside a frame's bounds or a tag that fails is `InvalidData`: the connection is over.
    pub fn next(&mut self, stream: &mut impl Read, cipher: &mut CipherState) -> io::Result<Vec<u8>> {
        loop {
            if self.buf.len() >= 2 {
                let len = usize::from(u16::from_le_bytes([self.buf[0], self.buf[1]]));
                if !(TAG_LEN..=MAX_JSON + TAG_LEN).contains(&len) {
                    return Err(invalid("frame length"));
                }
                if self.buf.len() >= 2 + len {
                    let plain = cipher.decrypt(&self.buf[2..2 + len]).map_err(|_| invalid("frame tag"));
                    self.buf.drain(..2 + len);
                    return plain;
                }
            }
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esphome_api::noise::{Initiator, Responder};

    fn pair() -> (esphome_api::noise::Transport, esphome_api::noise::Transport) {
        let prologue = Mode::Session.preamble();
        let (initiator, first) = Initiator::start_with(&prologue, &[7; 32], [1; 32]);
        let (second, tv) = Responder::respond_with(&prologue, &[7; 32], [2; 32], &first).unwrap();
        (initiator.finish(&second).unwrap(), tv)
    }

    #[test]
    fn preambles_name_their_mode_and_nothing_else_parses() {
        assert_eq!(Mode::Session.preamble(), *b"LRMT\x01\x01");
        assert_eq!(Mode::parse(b"LRMT\x01\x02"), Some(Mode::Pair));
        assert_eq!(Mode::parse(b"LRMT\x02\x01"), None);
        assert_eq!(Mode::parse(b"LRMT\x01\x03"), None);
        assert_eq!(Mode::parse(b"HTTP/1"), None);
    }

    #[test]
    fn frames_split_across_reads_come_out_whole_and_a_bad_tag_ends_the_stream() {
        let (mut phone, mut tv) = pair();
        let mut bytes = seal(&mut phone.send, br#"{"t":"ping","n":1}"#).unwrap();
        bytes.extend(seal(&mut phone.send, br#"{"t":"bye"}"#).unwrap());
        // One byte per read, as a slow link could deliver them.
        struct Trickle(Vec<u8>, usize);
        impl Read for Trickle {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.1 == self.0.len() {
                    return Ok(0);
                }
                out[0] = self.0[self.1];
                self.1 += 1;
                Ok(1)
            }
        }
        let mut stream = Trickle(bytes, 0);
        let mut reader = Reader::default();
        assert_eq!(reader.next(&mut stream, &mut tv.receive).unwrap(), br#"{"t":"ping","n":1}"#);
        assert_eq!(reader.next(&mut stream, &mut tv.receive).unwrap(), br#"{"t":"bye"}"#);
        assert_eq!(reader.next(&mut stream, &mut tv.receive).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);

        let mut forged = seal(&mut phone.send, b"{}").unwrap();
        forged[3] ^= 1;
        let err = Reader::default().next(&mut &forged[..], &mut tv.receive).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(seal(&mut phone.send, &[b' '; MAX_JSON + 1]).is_err());
    }
}
