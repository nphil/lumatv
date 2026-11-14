//! The part of CBOR the hub and the agent exchange: integer-keyed maps of numbers, text, bytes and
//! arrays. Encoding follows the Go agent's encoder (shortest integers, every float as 64 bits);
//! decoding is strict about the types of known fields and skips unknown ones.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Default)]
pub struct Enc {
    pub buf: Vec<u8>,
}

impl Enc {
    pub fn new() -> Self {
        Self::default()
    }

    fn head(&mut self, major: u8, value: u64) {
        let major = major << 5;
        if value < 24 {
            self.buf.push(major | value as u8);
        } else if value <= u64::from(u8::MAX) {
            self.buf.extend_from_slice(&[major | 24, value as u8]);
        } else if value <= u64::from(u16::MAX) {
            self.buf.push(major | 25);
            self.buf.extend_from_slice(&(value as u16).to_be_bytes());
        } else if value <= u64::from(u32::MAX) {
            self.buf.push(major | 26);
            self.buf.extend_from_slice(&(value as u32).to_be_bytes());
        } else {
            self.buf.push(major | 27);
            self.buf.extend_from_slice(&value.to_be_bytes());
        }
    }

    pub fn uint(&mut self, value: u64) {
        self.head(0, value);
    }

    pub fn int(&mut self, value: i64) {
        if value >= 0 {
            self.head(0, value as u64);
        } else {
            self.head(1, !(value as u64));
        }
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.head(2, value.len() as u64);
        self.buf.extend_from_slice(value);
    }

    pub fn text(&mut self, value: &str) {
        self.head(3, value.len() as u64);
        self.buf.extend_from_slice(value.as_bytes());
    }

    pub fn array(&mut self, len: usize) {
        self.head(4, len as u64);
    }

    pub fn map(&mut self, len: usize) {
        self.head(5, len as u64);
    }

    pub fn f64(&mut self, value: f64) {
        self.buf.push(0xfb);
        self.buf.extend_from_slice(&value.to_bits().to_be_bytes());
    }

    #[cfg(test)]
    pub fn bool(&mut self, value: bool) {
        self.buf.push(if value { 0xf5 } else { 0xf4 });
    }

    pub fn null(&mut self) {
        self.buf.push(0xf6);
    }

    /// Appends an already encoded item.
    pub fn raw(&mut self, item: &[u8]) {
        self.buf.extend_from_slice(item);
    }

    /// Opens a map whose size is known only once its entries are written; `end_map` fills it in.
    pub fn begin_map(&mut self) -> Map {
        self.buf.push(0xa0);
        Map { at: self.buf.len() - 1, len: 0 }
    }

    /// Writes the key of the next entry in `map`.
    pub fn key(&mut self, map: &mut Map, key: u64) {
        map.len += 1;
        self.uint(key);
    }

    pub fn end_map(&mut self, map: Map) {
        if map.len < 24 {
            self.buf[map.at] = 0xa0 | map.len as u8;
        } else {
            let mut head = Enc::new();
            head.map(map.len);
            self.buf.splice(map.at..=map.at, head.buf);
        }
    }
}

pub struct Map {
    at: usize,
    len: usize,
}

/// A bounds-checked reader over one encoded message.
pub struct Dec<'a> {
    buf: &'a [u8],
    pos: usize,
}

const MAX_DEPTH: usize = 16;
const BREAK: u8 = 0xff;

impl<'a> Dec<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn finished(&self) -> bool {
        self.pos == self.buf.len()
    }

    fn peek(&self) -> Result<u8> {
        self.buf.get(self.pos).copied().ok_or(Error("unexpected end of data"))
    }

    fn take(&mut self, n: u64) -> Result<&'a [u8]> {
        let n = usize::try_from(n).map_err(|_| Error("length out of range"))?;
        let end = self.pos.checked_add(n).filter(|&end| end <= self.buf.len());
        let end = end.ok_or(Error("unexpected end of data"))?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Reads an item's major type and argument; `None` is an indefinite length.
    fn head(&mut self) -> Result<(u8, Option<u64>)> {
        let first = self.take(1)?[0];
        let (major, info) = (first >> 5, first & 0x1f);
        let arg = match info {
            0..=23 => Some(u64::from(info)),
            24 => Some(u64::from(self.take(1)?[0])),
            25 => Some(u64::from(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))),
            26 => Some(u64::from(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))),
            27 => Some(u64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            31 if matches!(major, 2..=5) => None,
            _ => return Err(Error("malformed item")),
        };
        Ok((major, arg))
    }

    /// Consumes a `null` or `undefined`, reporting whether one was there.
    pub fn null(&mut self) -> Result<bool> {
        if matches!(self.peek()?, 0xf6 | 0xf7) {
            self.pos += 1;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn uint(&mut self) -> Result<u64> {
        match self.head()? {
            (0, Some(value)) => Ok(value),
            _ => Err(Error("expected an unsigned integer")),
        }
    }

    pub fn bool(&mut self) -> Result<bool> {
        match self.take(1)?[0] {
            0xf4 => Ok(false),
            0xf5 => Ok(true),
            _ => Err(Error("expected a boolean")),
        }
    }

    pub fn bytes(&mut self) -> Result<Vec<u8>> {
        self.string(2)
    }

    #[cfg(test)]
    pub fn text(&mut self) -> Result<String> {
        String::from_utf8(self.string(3)?).map_err(|_| Error("text is not UTF-8"))
    }

    fn string(&mut self, major: u8) -> Result<Vec<u8>> {
        match self.head()? {
            (m, Some(len)) if m == major => Ok(self.take(len)?.to_vec()),
            (m, None) if m == major => {
                let mut out = Vec::new();
                while self.peek()? != BREAK {
                    match self.head()? {
                        (m, Some(len)) if m == major => out.extend_from_slice(self.take(len)?),
                        _ => return Err(Error("malformed string chunk")),
                    }
                }
                self.pos += 1;
                Ok(out)
            }
            _ => Err(Error(if major == 2 { "expected bytes" } else { "expected text" })),
        }
    }

    /// Reads a map with integer keys, handing each key to `field`, which must consume its value.
    /// Text keys mean a different schema and are an error.
    pub fn map(&mut self, mut field: impl FnMut(&mut Self, Option<u64>) -> Result<()>) -> Result<()> {
        let len = match self.head()? {
            (5, len) => len,
            _ => return Err(Error("expected a map")),
        };
        let mut read = 0u64;
        loop {
            match len {
                Some(len) if read == len => break,
                None if self.peek()? == BREAK => {
                    self.pos += 1;
                    break;
                }
                _ => {}
            }
            let key = match self.head()? {
                (0, Some(key)) => Some(key),
                // A negative key matches no field.
                (1, Some(_)) => None,
                _ => return Err(Error("expected an integer key")),
            };
            field(self, key)?;
            read += 1;
        }
        Ok(())
    }

    /// Counts the items of an array, skipping them.
    pub fn array_len(&mut self) -> Result<u64> {
        let len = match self.head()? {
            (4, len) => len,
            _ => return Err(Error("expected an array")),
        };
        let mut read = 0u64;
        loop {
            match len {
                Some(len) if read == len => return Ok(read),
                None if self.peek()? == BREAK => {
                    self.pos += 1;
                    return Ok(read);
                }
                _ => self.skip()?,
            }
            read += 1;
        }
    }

    /// Returns the encoded bytes of the next item and moves past it.
    pub fn item(&mut self) -> Result<&'a [u8]> {
        let start = self.pos;
        self.skip()?;
        Ok(&self.buf[start..self.pos])
    }

    pub fn skip(&mut self) -> Result<()> {
        self.skip_at(0)
    }

    fn skip_at(&mut self, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Error("nested too deeply"));
        }
        let first = self.peek()?;
        if first >> 5 == 7 {
            self.pos += 1;
            let extra = match first & 0x1f {
                0..=23 => 0,
                24 => 1,
                25 => 2,
                26 => 4,
                27 => 8,
                _ => return Err(Error("malformed item")),
            };
            self.take(extra)?;
            return Ok(());
        }
        match self.head()? {
            (0 | 1, _) => {}
            (2 | 3, Some(len)) => {
                self.take(len)?;
            }
            (major @ (2 | 3), None) => {
                while self.peek()? != BREAK {
                    match self.head()? {
                        (m, Some(len)) if m == major => {
                            self.take(len)?;
                        }
                        _ => return Err(Error("malformed string chunk")),
                    }
                }
                self.pos += 1;
            }
            (major @ (4 | 5), len) => {
                let per_entry = if major == 5 { 2 } else { 1 };
                let mut read = 0u64;
                loop {
                    match len {
                        Some(len) if read == len => break,
                        None if self.peek()? == BREAK => {
                            self.pos += 1;
                            break;
                        }
                        _ => {}
                    }
                    for _ in 0..per_entry {
                        self.skip_at(depth + 1)?;
                    }
                    read += 1;
                }
            }
            (6, _) => self.skip_at(depth + 1)?,
            _ => return Err(Error("malformed item")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_take_the_shortest_form() {
        let mut e = Enc::new();
        e.uint(23);
        e.uint(24);
        e.uint(256);
        e.uint(65536);
        e.uint(1 << 32);
        e.int(-1);
        e.int(-25);
        assert_eq!(
            e.buf,
            [
                0x17, 0x18, 24, 0x19, 1, 0, 0x1a, 0, 1, 0, 0, 0x1b, 0, 0, 0, 1, 0, 0, 0, 0, 0x20, 0x38,
                24
            ]
        );
    }

    #[test]
    fn a_map_of_24_entries_gets_a_two_byte_head() {
        let mut e = Enc::new();
        let mut m = e.begin_map();
        for k in 0..24 {
            e.key(&mut m, k);
            e.bool(true);
        }
        e.end_map(m);
        assert_eq!(&e.buf[..3], [0xb8, 24, 0x00]);
        let mut d = Dec::new(&e.buf);
        let mut keys = Vec::new();
        d.map(|d, k| {
            keys.push(k.unwrap());
            assert!(d.bool()?);
            Ok(())
        })
        .unwrap();
        assert!(d.finished());
        assert_eq!(keys, (0..24).collect::<Vec<_>>());
    }

    #[test]
    fn skips_nested_and_indefinite_items() {
        // {_ 0: [_ h'01' , "ab"], 1: 1(1.5f16), 2: {3: simple(32)}} followed by 7
        let msg = [
            0xbf, 0x00, 0x9f, 0x41, 0x01, 0x62, b'a', b'b', 0xff, 0x01, 0xc1, 0xf9, 0x3e, 0x00, 0x02,
            0xa1, 0x03, 0xf8, 0x20, 0xff, 0x07,
        ];
        let mut d = Dec::new(&msg);
        d.skip().unwrap();
        assert_eq!(d.uint(), Ok(7));
        assert!(d.finished());
    }

    #[test]
    fn rejects_truncated_text_keys_and_deep_nesting() {
        assert!(Dec::new(&[0x62, b'a']).skip().is_err());
        assert!(Dec::new(&[0xa1, 0x61, b'a', 0x00]).map(|d, _| d.skip()).is_err());
        let deep = [0x81; 40];
        assert_eq!(Dec::new(&deep).skip(), Err(Error("nested too deeply")));
    }
}
