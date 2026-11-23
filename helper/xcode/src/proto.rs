//! The records between the engine and `lumatv-venc xcode`: the wire of `venc/xcode_proto.h`. Every record is a 24-byte little-endian header and
//! then `size` payload bytes. The engine writes `XCFG`, `XAUN`, `XIDR` and `XBPS`; the child answers `XRDY`, `XOUT` and `XEVT`. [`read_record`] is the
//! engine's side; [`read_command`] and the `encode_*` functions of the child's records are for fake children in tests.

use std::fmt;
use std::io::{self, Read};

pub const HEADER_LEN: usize = 24;
pub const VERSION: u32 = 1;
/// The largest payload a record may announce; a bigger header is a protocol error.
pub const MAX_PAYLOAD: usize = 4 << 20;
pub const CFG_LEN: usize = 64;
pub const READY_LEN: usize = 24;
pub const EVENT_LEN: usize = 32;

pub const CONFIG: [u8; 4] = *b"XCFG";
pub const READY: [u8; 4] = *b"XRDY";
pub const AU: [u8; 4] = *b"XAUN";
pub const OUT: [u8; 4] = *b"XOUT";
pub const IDR: [u8; 4] = *b"XIDR";
pub const BITRATE: [u8; 4] = *b"XBPS";
pub const EVENT: [u8; 4] = *b"XEVT";

/// XAUN: a random access point; XOUT: an IDR.
pub const FLAG_KEY: u32 = 1;

pub const CODEC_MPEG2: u32 = 1;
pub const CODEC_H264: u32 = 2;

/// `Cfg::flags`: constant bit rate.
pub const CFG_CBR: u32 = 1;

/// `Evt::kind`: once a second; x: access units in, y: units out, a: pictures dropped, b: last decode ms, c: last encode tenths of ms.
pub const EVT_STATS: u32 = 1;
/// The first picture: a, b: its size, c: milliseconds since the first access unit.
pub const EVT_FIRST: u32 = 2;
/// A failure: a is the exit status that follows, the text says what.
pub const EVT_ERROR: u32 = 3;

/// `LX_VENC_H264_PROFILE_*` in `venc/venc_abi.h`.
pub const PROFILE_HIGH: u32 = 0;
pub const PROFILE_MAIN: u32 = 1;
pub const PROFILE_BASELINE: u32 = 2;

/// The child's exit statuses (`XC_EXIT_*`).
pub const EXIT_OK: i32 = 0;
pub const EXIT_CONFIG: i32 = 2;
pub const EXIT_SETUP: i32 = 3;
pub const EXIT_DECODER: i32 = 4;
pub const EXIT_STALLED: i32 = 5;
pub const EXIT_ENCODER: i32 = 6;
pub const EXIT_SHAPE: i32 = 7;
/// A call into the hardware did not return.
pub const EXIT_WEDGED: i32 = 75;

/// Error text longer than this is cut: it is shown in a toast and kept in memory.
const TEXT_MAX: usize = 512;

/// Reads little-endian fields; a short input reads as zeros (callers check the length first).
struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        match self.0.split_first_chunk::<N>() {
            Some((head, rest)) => {
                self.0 = rest;
                *head
            }
            None => [0; N],
        }
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }
}

/// Writes little-endian fields into a buffer of exactly the right size.
struct Wr<'a>(&'a mut [u8]);

impl Wr<'_> {
    fn put(&mut self, src: &[u8]) {
        let buf = std::mem::take(&mut self.0);
        if let Some((dst, rest)) = buf.split_at_mut_checked(src.len()) {
            dst.copy_from_slice(src);
            self.0 = rest;
        }
    }

    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.put(&v.to_le_bytes());
    }
}

/// The 24-byte header of every record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub magic: [u8; 4],
    pub flags: u32,
    pub pts: u64,
    /// Payload bytes after the header.
    pub size: u32,
    pub aux: u32,
}

impl Head {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        let mut w = Wr(&mut out);
        w.put(&self.magic);
        w.u32(self.flags);
        w.u64(self.pts);
        w.u32(self.size);
        w.u32(self.aux);
        out
    }

    pub fn decode(bytes: &[u8; HEADER_LEN]) -> Self {
        let mut r = Rd(bytes);
        Self { magic: r.take(), flags: r.u32(), pts: r.u64(), size: r.u32(), aux: r.u32() }
    }
}

/// The job, as the child is told it (`struct xc_cfg`, 64 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cfg {
    pub version: u32,
    pub codec: u32,
    /// The largest picture the source sends; sizes the picture buffers.
    pub src_w: u32,
    pub src_h: u32,
    pub out_w: u32,
    pub out_h: u32,
    pub keep_every: u32,
    pub fps: u32,
    pub kbps: u32,
    pub gop: u32,
    pub flags: u32,
    pub profile: u32,
    pub ncap: u32,
    pub nout: u32,
    pub out_size: u32,
    pub reserved: u32,
}

impl Cfg {
    pub fn encode(&self) -> [u8; CFG_LEN] {
        let mut out = [0u8; CFG_LEN];
        let mut w = Wr(&mut out);
        for v in [
            self.version,
            self.codec,
            self.src_w,
            self.src_h,
            self.out_w,
            self.out_h,
            self.keep_every,
            self.fps,
            self.kbps,
            self.gop,
            self.flags,
            self.profile,
            self.ncap,
            self.nout,
            self.out_size,
            self.reserved,
        ] {
            w.u32(v);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != CFG_LEN {
            return None;
        }
        let mut r = Rd(bytes);
        Some(Self {
            version: r.u32(),
            codec: r.u32(),
            src_w: r.u32(),
            src_h: r.u32(),
            out_w: r.u32(),
            out_h: r.u32(),
            keep_every: r.u32(),
            fps: r.u32(),
            kbps: r.u32(),
            gop: r.u32(),
            flags: r.u32(),
            profile: r.u32(),
            ncap: r.u32(),
            nout: r.u32(),
            out_size: r.u32(),
            reserved: r.u32(),
        })
    }
}

/// The child is up (`struct xc_ready`, 24 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ready {
    pub version: u32,
    pub max_w: u32,
    pub max_h: u32,
    pub out_size: u32,
    pub ncap: u32,
    /// What the child holds of the TV's contiguous memory.
    pub hma_bytes: u32,
}

impl Ready {
    pub fn encode(&self) -> [u8; READY_LEN] {
        let mut out = [0u8; READY_LEN];
        let mut w = Wr(&mut out);
        for v in [self.version, self.max_w, self.max_h, self.out_size, self.ncap, self.hma_bytes] {
            w.u32(v);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != READY_LEN {
            return None;
        }
        let mut r = Rd(bytes);
        Some(Self { version: r.u32(), max_w: r.u32(), max_h: r.u32(), out_size: r.u32(), ncap: r.u32(), hma_bytes: r.u32() })
    }
}

/// A report from the child (`struct xc_event`, 32 bytes, then text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evt {
    pub kind: u32,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub x: u64,
    pub y: u64,
    /// For `EVT_ERROR`, what failed; cut to 512 bytes.
    pub text: String,
}

impl Evt {
    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < EVENT_LEN {
            return None;
        }
        let mut r = Rd(bytes);
        let (kind, a, b, c, x, y) = (r.u32(), r.u32(), r.u32(), r.u32(), r.u64(), r.u64());
        let text = r.0.get(..r.0.len().min(TEXT_MAX)).unwrap_or_default();
        Some(Self { kind, a, b, c, x, y, text: String::from_utf8_lossy(text).trim_end().to_owned() })
    }
}

/// One unit out of the child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub key: bool,
    pub pts: u64,
    /// The encoder's own tenths of a millisecond for the picture.
    pub aux: u32,
    pub data: Vec<u8>,
}

/// What the child sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Ready(Ready),
    Out(Unit),
    Event(Evt),
}

/// What the engine sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Config(Cfg),
    Au { key: bool, pts: u64, data: Vec<u8> },
    Idr,
    Bitrate(u32),
}

/// Why a record could not be read.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// A record the other side does not send.
    Magic([u8; 4]),
    /// A payload size the record cannot have, or more than [`MAX_PAYLOAD`].
    Size { magic: [u8; 4], size: u32 },
    /// The stream ended inside a record.
    Truncated,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "read failed: {e}"),
            Self::Magic(m) => write!(f, "unknown record {:?}", String::from_utf8_lossy(m)),
            Self::Size { magic, size } => write!(f, "record {:?} of {size} bytes", String::from_utf8_lossy(magic)),
            Self::Truncated => f.write_str("the stream ended inside a record"),
        }
    }
}

impl std::error::Error for Error {}

/// The payload sizes a record may have by its magic, or `None` for a record the side does not send.
type Rule = fn(&[u8; 4]) -> Option<(usize, usize)>;

fn from_child(magic: &[u8; 4]) -> Option<(usize, usize)> {
    match *magic {
        READY => Some((READY_LEN, READY_LEN)),
        OUT => Some((0, MAX_PAYLOAD)),
        EVENT => Some((EVENT_LEN, MAX_PAYLOAD)),
        _ => None,
    }
}

fn from_engine(magic: &[u8; 4]) -> Option<(usize, usize)> {
    match *magic {
        CONFIG => Some((CFG_LEN, CFG_LEN)),
        AU => Some((0, MAX_PAYLOAD)),
        IDR | BITRATE => Some((0, 0)),
        _ => None,
    }
}

fn fill<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> Result<(), Error> {
    r.read_exact(buf).map_err(|e| if e.kind() == io::ErrorKind::UnexpectedEof { Error::Truncated } else { Error::Io(e) })
}

/// One header and its payload, or `None` at a clean end of the stream. The header is checked before the payload is allocated, so garbage cannot
/// make the reader take more than a record's worth of memory.
fn read_frame<R: Read + ?Sized>(r: &mut R, rule: Rule) -> Result<Option<(Head, Vec<u8>)>, Error> {
    let mut first = [0u8; 1];
    loop {
        match r.read(&mut first) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Error::Io(e)),
        }
    }
    let mut rest = [0u8; HEADER_LEN - 1];
    fill(r, &mut rest)?;
    let mut bytes = [0u8; HEADER_LEN];
    let mut w = Wr(&mut bytes);
    w.put(&first);
    w.put(&rest);
    let head = Head::decode(&bytes);
    let Some((min, max)) = rule(&head.magic) else {
        return Err(Error::Magic(head.magic));
    };
    let size = head.size as usize;
    if size < min || size > max.min(MAX_PAYLOAD) {
        return Err(Error::Size { magic: head.magic, size: head.size });
    }
    let mut body = vec![0u8; size];
    fill(r, &mut body)?;
    Ok(Some((head, body)))
}

/// The next record the child sent, or `None` when it closed its output between records.
pub fn read_record<R: Read + ?Sized>(r: &mut R) -> Result<Option<Record>, Error> {
    let Some((head, body)) = read_frame(r, from_child)? else {
        return Ok(None);
    };
    let bad = Error::Size { magic: head.magic, size: head.size };
    Ok(Some(match head.magic {
        READY => Record::Ready(Ready::decode(&body).ok_or(bad)?),
        EVENT => Record::Event(Evt::decode(&body).ok_or(bad)?),
        _ => Record::Out(Unit { key: head.flags & FLAG_KEY != 0, pts: head.pts, aux: head.aux, data: body }),
    }))
}

/// The next record the engine sent: what a fake child reads.
pub fn read_command<R: Read + ?Sized>(r: &mut R) -> Result<Option<Command>, Error> {
    let Some((head, body)) = read_frame(r, from_engine)? else {
        return Ok(None);
    };
    Ok(Some(match head.magic {
        CONFIG => Command::Config(Cfg::decode(&body).ok_or(Error::Size { magic: head.magic, size: head.size })?),
        AU => Command::Au { key: head.flags & FLAG_KEY != 0, pts: head.pts, data: body },
        IDR => Command::Idr,
        _ => Command::Bitrate(head.aux),
    }))
}

fn head(magic: [u8; 4], flags: u32, pts: u64, size: usize, aux: u32) -> [u8; HEADER_LEN] {
    // a payload is at most MAX_PAYLOAD, which a u32 holds
    Head { magic, flags, pts, size: u32::try_from(size).unwrap_or(u32::MAX), aux }.encode()
}

/// `XCFG` and its configuration: the first record the child is sent.
pub fn encode_config(cfg: &Cfg) -> Vec<u8> {
    let mut out = head(CONFIG, 0, 0, CFG_LEN, 0).to_vec();
    out.extend_from_slice(&cfg.encode());
    out
}

/// The header of an `XAUN`; the access unit's bytes follow it.
pub fn encode_au_head(key: bool, pts: u64, len: usize) -> [u8; HEADER_LEN] {
    head(AU, u32::from(key) * FLAG_KEY, pts, len, 0)
}

/// `XIDR`: the next unit is an IDR.
pub fn encode_idr() -> [u8; HEADER_LEN] {
    head(IDR, 0, 0, 0, 0)
}

/// `XBPS`: a new target bit rate in kbit/s.
pub fn encode_bitrate(kbps: u32) -> [u8; HEADER_LEN] {
    head(BITRATE, 0, 0, 0, kbps)
}

pub fn encode_ready(ready: &Ready) -> Vec<u8> {
    let mut out = head(READY, 0, 0, READY_LEN, 0).to_vec();
    out.extend_from_slice(&ready.encode());
    out
}

pub fn encode_out(key: bool, pts: u64, aux: u32, data: &[u8]) -> Vec<u8> {
    let mut out = head(OUT, u32::from(key) * FLAG_KEY, pts, data.len(), aux).to_vec();
    out.extend_from_slice(data);
    out
}

pub fn encode_event(evt: &Evt) -> Vec<u8> {
    let mut out = head(EVENT, 0, 0, EVENT_LEN + evt.text.len(), 0).to_vec();
    let mut fixed = [0u8; EVENT_LEN];
    let mut w = Wr(&mut fixed);
    for v in [evt.kind, evt.a, evt.b, evt.c] {
        w.u32(v);
    }
    w.u64(evt.x);
    w.u64(evt.y);
    out.extend_from_slice(&fixed);
    out.extend_from_slice(evt.text.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that hands out one byte per call: records must survive any split of the stream.
    struct OneByte<'a>(&'a [u8]);

    impl Read for OneByte<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match (self.0.split_first(), buf.first_mut()) {
                (Some((byte, rest)), Some(slot)) => {
                    *slot = *byte;
                    self.0 = rest;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    #[test]
    fn the_header_is_the_bytes_the_c_side_asserts() {
        // venc/tests/xcode_test.c, test_wire
        let golden: [u8; 24] = [b'X', b'A', b'U', b'N', 1, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 3, 0, 0, 0, 7, 0, 0, 0];
        let head = Head { magic: AU, flags: FLAG_KEY, pts: 0x0102_0304_0506_0708, size: 3, aux: 7 };
        assert_eq!(head.encode(), golden);
        assert_eq!(Head::decode(&golden), head);
        assert_eq!(encode_au_head(true, 0x0102_0304_0506_0708, 3)[..20], golden[..20]);
    }

    #[test]
    fn the_configuration_is_64_little_endian_words() {
        let cfg = Cfg {
            version: 1,
            codec: CODEC_MPEG2,
            src_w: 1920,
            src_h: 1088,
            out_w: 1280,
            out_h: 720,
            keep_every: 2,
            fps: 30,
            kbps: 3500,
            gop: 60,
            flags: CFG_CBR,
            profile: PROFILE_MAIN,
            ncap: 0,
            nout: 0,
            out_size: 0,
            reserved: 0,
        };
        let mut want = Vec::new();
        want.extend_from_slice(b"XCFG");
        want.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0]);
        for word in [
            [1u8, 0, 0, 0],
            [1, 0, 0, 0],
            [0x80, 0x07, 0, 0],
            [0x40, 0x04, 0, 0],
            [0x00, 0x05, 0, 0],
            [0xD0, 0x02, 0, 0],
            [2, 0, 0, 0],
            [30, 0, 0, 0],
            [0xAC, 0x0D, 0, 0],
            [60, 0, 0, 0],
            [1, 0, 0, 0],
            [1, 0, 0, 0],
            [0; 4],
            [0; 4],
            [0; 4],
            [0; 4],
        ] {
            want.extend_from_slice(&word);
        }
        assert_eq!(encode_config(&cfg), want);
        assert_eq!(want.len(), HEADER_LEN + CFG_LEN);
        assert_eq!(Cfg::decode(&cfg.encode()), Some(cfg));
        assert_eq!(read_command(&mut want.as_slice()).unwrap(), Some(Command::Config(cfg)));
    }

    #[test]
    fn control_records_are_a_bare_header() {
        assert_eq!(&encode_idr()[..4], b"XIDR");
        assert_eq!(encode_idr()[16..20], [0, 0, 0, 0]);
        let bps = encode_bitrate(0x0102);
        assert_eq!(&bps[..4], b"XBPS");
        assert_eq!(bps[20..], [2, 1, 0, 0]);
        assert_eq!(read_command(&mut bps.as_slice()).unwrap(), Some(Command::Bitrate(0x0102)));
        assert_eq!(read_command(&mut encode_idr().as_slice()).unwrap(), Some(Command::Idr));
    }

    #[test]
    fn a_stream_of_records_reads_the_same_one_byte_at_a_time() {
        let ready = Ready { version: VERSION, max_w: 1920, max_h: 1088, out_size: 1_572_864, ncap: 6, hma_bytes: 12_533_760 };
        let error = Evt { kind: EVT_ERROR, a: 4, text: "decoder: no memory".into(), ..Evt::default() };
        let stats = Evt { kind: EVT_STATS, a: 1, b: 2, c: 3, x: 4, y: 5, text: String::new() };
        let mut wire = Vec::new();
        wire.extend(encode_ready(&ready));
        wire.extend(encode_out(true, 90_000, 17, &[0, 0, 0, 1, 0x67]));
        wire.extend(encode_out(false, 93_003, 9, &[]));
        wire.extend(encode_event(&stats));
        wire.extend(encode_event(&error));
        let want = vec![
            Record::Ready(ready),
            Record::Out(Unit { key: true, pts: 90_000, aux: 17, data: vec![0, 0, 0, 1, 0x67] }),
            Record::Out(Unit { key: false, pts: 93_003, aux: 9, data: Vec::new() }),
            Record::Event(stats),
            Record::Event(error),
        ];
        for mut r in [Box::new(wire.as_slice()) as Box<dyn Read>, Box::new(OneByte(&wire))] {
            let mut got = Vec::new();
            while let Some(rec) = read_record(&mut r).unwrap() {
                got.push(rec);
            }
            assert_eq!(got, want);
        }
    }

    #[test]
    fn error_text_is_cut_and_read_lossily() {
        let long = Evt { kind: EVT_ERROR, text: "x".repeat(2000), ..Evt::default() };
        let Some(Record::Event(got)) = read_record(&mut encode_event(&long).as_slice()).unwrap() else { panic!("not an event") };
        assert_eq!(got.text.len(), TEXT_MAX);
        let mut raw = encode_event(&Evt { kind: EVT_ERROR, text: "ab".into(), ..Evt::default() });
        raw.pop();
        raw.pop();
        raw.extend_from_slice(&[0xFF, 0xFE]);
        let Some(Record::Event(got)) = read_record(&mut raw.as_slice()).unwrap() else { panic!("not an event") };
        assert_eq!(got.text, "\u{FFFD}\u{FFFD}");
    }

    #[test]
    fn garbage_is_an_error_and_never_a_big_allocation() {
        // a record the child does not send
        assert!(matches!(read_record(&mut encode_idr().as_slice()), Err(Error::Magic(m)) if m == IDR));
        assert!(matches!(read_record(&mut [0x5A; 64].as_slice()), Err(Error::Magic(_))));
        // a size above the protocol's: refused from the header alone, with nothing after it to read
        let huge = Head { magic: OUT, flags: 0, pts: 0, size: MAX_PAYLOAD as u32 + 1, aux: 0 }.encode();
        assert!(matches!(read_record(&mut huge.as_slice()), Err(Error::Size { size, .. }) if size as usize == MAX_PAYLOAD + 1));
        let giant = Head { magic: EVENT, flags: 0, pts: 0, size: u32::MAX, aux: 0 }.encode();
        assert!(matches!(read_record(&mut giant.as_slice()), Err(Error::Size { .. })));
        // a ready record of the wrong size, an event too short for its struct
        let short_ready = Head { magic: READY, flags: 0, pts: 0, size: 8, aux: 0 }.encode();
        assert!(matches!(read_record(&mut short_ready.as_slice()), Err(Error::Size { .. })));
        let short_event = Head { magic: EVENT, flags: 0, pts: 0, size: 31, aux: 0 }.encode();
        assert!(matches!(read_record(&mut short_event.as_slice()), Err(Error::Size { .. })));
        // the stream ends inside a header, and inside a payload
        assert!(matches!(read_record(&mut encode_ready(&Ready::default())[..10].as_ref()), Err(Error::Truncated)));
        assert!(matches!(read_record(&mut encode_out(true, 1, 1, &[1, 2, 3])[..HEADER_LEN + 2].as_ref()), Err(Error::Truncated)));
        // nothing at all is a clean end
        assert!(read_record(&mut [].as_slice()).unwrap().is_none());
    }
}
