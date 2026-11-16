//! Shared engine client for `gfn/feeder/src/gf_proto.h`; see `../PROTOCOL.md`.
//! Media writes borrow the access unit and use a stack-allocated writev. Replies run on one reader thread.
use std::io::{self, IoSlice, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const PROTO_VERSION: u32 = 2;
pub const MAX_PAYLOAD: u32 = 8 << 20;

pub mod msg {
    pub const LOAD: u8 = 1;
    pub const VIDEO: u8 = 2;
    pub const AUDIO: u8 = 3;
    pub const FLUSH: u8 = 4;
    pub const UNLOAD: u8 = 5;
    pub const STATE: u8 = 6;
    pub const HDR: u8 = 7;
    pub const PING: u8 = 8;
    pub const APP: u8 = 9;
    pub const TEXTURE: u8 = 10;
    pub const PIPE: u8 = 11;
    pub const SINK: u8 = 12;
    pub const LOAD_OPUS51: u8 = 13;
    pub const LOADED: u8 = 16;
    pub const EVENT: u8 = 17;
    pub const ACCEPT: u8 = 18;
    pub const DROP: u8 = 19;
    pub const STATUS: u8 = 20;
    pub const ERROR: u8 = 21;
    pub const PONG: u8 = 22;
    pub const HELLO: u8 = 23;
    pub const KEYFRAME: u8 = 24;
}

pub mod codec {
    pub const H264: u32 = 1;
    pub const H265: u32 = 2;
    pub const AV1: u32 = 4;
}

pub mod audio {
    pub const NONE: u32 = 0;
    pub const OPUS: u32 = 1;
    pub const PCM_S16LE: u32 = 2;
}

pub mod pts_unit {
    pub const NS: i32 = 1;
    pub const US: i32 = 1000;
    pub const MS: i32 = 1_000_000;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub kind: u8,
    pub flags: u8,
    pub reserved: u16,
    pub length: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Hello {
    pub version: u32,
    pub library: u32,
    pub pid: u32,
    pub window: u32,
    pub window_id: [u8; 32],
    pub library_name: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hdr {
    pub display_primaries_x0: u32,
    pub display_primaries_y0: u32,
    pub display_primaries_x1: u32,
    pub display_primaries_y1: u32,
    pub display_primaries_x2: u32,
    pub display_primaries_y2: u32,
    pub white_point_x: u32,
    pub white_point_y: u32,
    pub max_display_mastering_luminance: u32,
    pub min_display_mastering_luminance: u32,
    pub max_content_light_level: u32,
    pub max_pic_average_light_level: u32,
    pub transfer_characteristics: u32,
    pub color_primaries: u32,
    pub matrix_coeffs: u32,
}

impl Hdr {
    /// HDR10 as GFN streams it (docs/research/gfn-444-vrr-result.md, "HDR"): BT.2020 primaries, D65, PQ; the mastering luminance and
    /// light levels are what OpenNOW's session request declares for the display (1000 nits peak, 400 average, 0.0001 floor).
    pub fn hdr10(max_nits: u32, max_average_nits: u32) -> Self {
        Self {
            display_primaries_x0: 13250,
            display_primaries_y0: 34500,
            display_primaries_x1: 7500,
            display_primaries_y1: 3000,
            display_primaries_x2: 34000,
            display_primaries_y2: 16000,
            white_point_x: 15635,
            white_point_y: 16450,
            max_display_mastering_luminance: max_nits * 10000,
            min_display_mastering_luminance: 1,
            max_content_light_level: max_nits,
            max_pic_average_light_level: max_average_nits,
            transfer_characteristics: 16,
            color_primaries: 9,
            matrix_coeffs: 9,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Load {
    pub codec: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub audio: u32,
    pub channels: u32,
    pub sample_rate: u32,
    pub hdr: u32,
    pub hdr_info: Hdr,
    pub drop_threshold: i32,
    pub pts_unit: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOpus51 {
    pub load: Load,
    pub streams: u8,
    pub coupled: u8,
    pub mapping: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MediaHead {
    pub pts_ns: i64,
    pub key: u8,
    pub pad: [u8; 7],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Event {
    pub kind: i32,
    pub pad: i32,
    pub num: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Accept {
    pub pts_ns: i64,
    pub queue: i32,
    pub wait_us: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Drop {
    pub pts_ns: i64,
    pub reason: i32,
    pub pad: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    pub video_fed: u64,
    pub video_dropped: u64,
    pub audio_fed: u64,
    pub audio_dropped: u64,
    pub full_retries: u64,
    pub queue_min: u32,
    pub queue_max: u32,
    pub queue_mean_x100: u32,
    pub queue_p99: u32,
    pub wait_max_us: u32,
    pub wait_p99_us: u32,
    pub buffer_low: u32,
    pub buffer_full: u32,
    pub errors: u32,
    pub loaded: u32,
    pub playing: u32,
    pub cma_free_kb: u32,
    pub mem_available_kb: u32,
    pub rss_kb: u32,
    pub audio_avail: u32,
}

const _: () = assert!(std::mem::size_of::<Frame>() == 8);
const _: () = assert!(std::mem::size_of::<Hello>() == 112);
const _: () = assert!(std::mem::size_of::<Hdr>() == 60);
const _: () = assert!(std::mem::size_of::<Load>() == 100);
const _: () = assert!(std::mem::size_of::<LoadOpus51>() == 108);
const _: () = assert!(std::mem::size_of::<MediaHead>() == 16);
const _: () = assert!(std::mem::size_of::<Event>() == 16);
const _: () = assert!(std::mem::size_of::<Accept>() == 16);
const _: () = assert!(std::mem::size_of::<Drop>() == 16);
const _: () = assert!(std::mem::size_of::<Status>() == 104);
const _: () = assert!(cfg!(target_endian = "little"));

fn as_bytes<T: Copy>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

fn read_struct<T: Copy>(b: &[u8]) -> Option<T> {
    if b.len() < std::mem::size_of::<T>() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(b.as_ptr() as *const T) })
}

/// Partial writes and signals preserve the frame boundary without copying the access unit.
fn write_all_vectored<const N: usize>(stream: &mut impl Write, parts: &[&[u8]; N]) -> io::Result<()> {
    let mut slices = parts.map(IoSlice::new);
    let mut remaining = &mut slices[..];
    while !remaining.is_empty() {
        match stream.write_vectored(remaining) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "the feeder socket took nothing")),
            Ok(n) => IoSlice::advance_slices(&mut remaining, n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn c_string(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// What the feeder said, decoded.
#[derive(Debug, Clone)]
pub enum FeederMessage {
    Hello { version: u32, library: bool, pid: u32, own_window: bool, window_id: String, library_name: String },
    Loaded { ok: bool, text: String },
    Event { kind: i32, num: i64, text: String },
    Accept(Accept),
    Drop(Drop),
    Status(Status),
    Error { code: i32, text: String },
    Pong { echo: i64, feeder_ns: i64 },
    Keyframe,
    Unknown { kind: u8, length: u32 },
}

pub fn decode(kind: u8, p: &[u8]) -> FeederMessage {
    match kind {
        msg::HELLO => match read_struct::<Hello>(p) {
            Some(h) if p.len() == std::mem::size_of::<Hello>() => FeederMessage::Hello {
                version: h.version,
                library: h.library != 0,
                pid: h.pid,
                own_window: h.window != 0,
                window_id: c_string(&h.window_id),
                library_name: c_string(&h.library_name),
            },
            _ => FeederMessage::Unknown { kind, length: p.len() as u32 },
        },
        msg::LOADED if p.len() >= 4 => FeederMessage::Loaded {
            ok: i32::from_le_bytes([p[0], p[1], p[2], p[3]]) == 1,
            text: String::from_utf8_lossy(&p[4..]).into_owned(),
        },
        msg::EVENT => match read_struct::<Event>(p) {
            Some(e) => FeederMessage::Event { kind: e.kind, num: e.num, text: String::from_utf8_lossy(&p[16..]).into_owned() },
            None => FeederMessage::Unknown { kind, length: p.len() as u32 },
        },
        msg::ACCEPT => read_struct::<Accept>(p).map_or(FeederMessage::Unknown { kind, length: p.len() as u32 }, FeederMessage::Accept),
        msg::DROP => read_struct::<Drop>(p).map_or(FeederMessage::Unknown { kind, length: p.len() as u32 }, FeederMessage::Drop),
        msg::STATUS => read_struct::<Status>(p).map_or(FeederMessage::Unknown { kind, length: p.len() as u32 }, FeederMessage::Status),
        msg::ERROR if p.len() >= 4 => FeederMessage::Error {
            code: i32::from_le_bytes([p[0], p[1], p[2], p[3]]),
            text: String::from_utf8_lossy(&p[4..]).into_owned(),
        },
        msg::PONG if p.len() >= 16 => FeederMessage::Pong {
            echo: i64::from_le_bytes(p[0..8].try_into().unwrap()),
            feeder_ns: i64::from_le_bytes(p[8..16].try_into().unwrap()),
        },
        msg::KEYFRAME if p.is_empty() => FeederMessage::Keyframe,
        _ => FeederMessage::Unknown { kind, length: p.len() as u32 },
    }
}

/// The writing half; the reader thread owns a clone of the stream.
pub struct FeederClient {
    stream: UnixStream,
    reader: Option<JoinHandle<()>>,
}

impl FeederClient {
    /// Connects, retrying until `timeout` (the feeder may still be starting), and reads the HELLO before handing the socket over.
    pub fn connect(path: &Path, timeout: Duration) -> io::Result<(Self, FeederMessage)> {
        let end = Instant::now() + timeout;
        let stream = loop {
            match UnixStream::connect(path) {
                Ok(s) => break s,
                Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) && Instant::now() < end => {
                    thread::sleep(Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())));
                }
                Err(e) => return Err(io::Error::new(e.kind(), format!("feeder socket {}: {e}", path.display()))),
            }
        };
        stream.set_read_timeout(Some(end.saturating_duration_since(Instant::now()).max(Duration::from_millis(1))))?;
        let mut s = Self { stream, reader: None };
        let (kind, payload) = s.read_frame()?;
        let hello = decode(kind, &payload);
        if !matches!(hello, FeederMessage::Hello { version: PROTO_VERSION, .. }) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the feeder did not greet with protocol 2"));
        }
        s.stream.set_read_timeout(None)?;
        s.stream.set_write_timeout(Some(Duration::from_millis(500)))?;
        Ok((s, hello))
    }

    fn read_frame(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let mut head = [0u8; 8];
        self.stream.read_exact(&mut head)?;
        let f: Frame = read_struct(&head).unwrap();
        if f.length > MAX_PAYLOAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("a {} byte message from the feeder", f.length)));
        }
        let mut payload = vec![0u8; f.length as usize];
        self.stream.read_exact(&mut payload)?;
        Ok((f.kind, payload))
    }

    /// Starts the reader thread; `on_message` runs on it until the socket ends, when it gets `None`.
    pub fn spawn_reader(&mut self, mut on_message: impl FnMut(Option<FeederMessage>) + Send + 'static) -> io::Result<()> {
        if self.reader.is_some() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "the feeder already has a reader"));
        }
        let mut stream = self.stream.try_clone()?;
        self.reader = Some(
            thread::Builder::new().name("lumagame-feeder".to_owned()).spawn(move || {
                let mut head = [0u8; 8];
                let mut payload = Vec::with_capacity(512);
                loop {
                    if stream.read_exact(&mut head).is_err() {
                        break;
                    }
                    let f: Frame = read_struct(&head).unwrap();
                    if f.length > MAX_PAYLOAD {
                        break;
                    }
                    payload.resize(f.length as usize, 0);
                    if stream.read_exact(&mut payload).is_err() {
                        break;
                    }
                    on_message(Some(decode(f.kind, &payload)));
                }
                on_message(None);
            })?,
        );
        Ok(())
    }

    pub fn send(&mut self, kind: u8, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_PAYLOAD as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a message larger than the feeder takes"));
        }
        let f = Frame { kind, flags: 0, reserved: 0, length: payload.len() as u32 };
        write_all_vectored(&mut self.stream, &[as_bytes(&f), payload])
    }

    pub fn send_media(&mut self, kind: u8, pts_ns: i64, key: bool, data: &[u8]) -> io::Result<()> {
        if !matches!(kind, msg::VIDEO | msg::AUDIO) || data.len() > (MAX_PAYLOAD - 16) as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "an access unit larger than the feeder takes"));
        }
        let f = Frame { kind, flags: 0, reserved: 0, length: (16 + data.len()) as u32 };
        let h = MediaHead { pts_ns, key: u8::from(key), pad: [0; 7] };
        write_all_vectored(&mut self.stream, &[as_bytes(&f), as_bytes(&h), data])
    }

    pub fn send_load(&mut self, load: &Load) -> io::Result<()> {
        self.send(msg::LOAD, as_bytes(load))
    }

    pub fn send_load_opus51(&mut self, load: &LoadOpus51) -> io::Result<()> {
        self.send(msg::LOAD_OPUS51, as_bytes(load))
    }

    pub fn send_hdr(&mut self, hdr: &Hdr) -> io::Result<()> {
        self.send(msg::HDR, as_bytes(hdr))
    }

    pub fn send_state(&mut self, foreground: bool) -> io::Result<()> {
        self.send(msg::STATE, &[u8::from(foreground)])
    }

    /// Reports loss. The feeder holds video and requests a keyframe; it does not flush NDL.
    pub fn send_flush(&mut self) -> io::Result<()> {
        self.send(msg::FLUSH, &[])
    }

    pub fn send_unload(&mut self) -> io::Result<()> {
        self.send(msg::UNLOAD, &[])
    }

    pub fn send_ping(&mut self, echo: i64) -> io::Result<()> {
        self.send(msg::PING, &echo.to_le_bytes())
    }

    /// Closes the socket; the reader thread ends with it.
    pub fn close(self) {}
}

impl std::ops::Drop for FeederClient {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_decode() {
        let mut h = Hello { version: PROTO_VERSION, library: 1, pid: 7, window: 0, window_id: [0; 32], library_name: [0; 64] };
        h.window_id[.."_Window_Id_9".len()].copy_from_slice(b"_Window_Id_9");
        match decode(msg::HELLO, as_bytes(&h)) {
            FeederMessage::Hello { library, pid, own_window, window_id, .. } => {
                assert!(library && !own_window && pid == 7 && window_id == "_Window_Id_9");
            }
            other => panic!("{other:?}"),
        }
        let mut p = 1i32.to_le_bytes().to_vec();
        p.extend_from_slice(b"_Window_Id_9");
        assert!(matches!(decode(msg::LOADED, &p), FeederMessage::Loaded { ok: true, .. }));
        let st = Status { video_fed: 120, loaded: 1, ..Default::default() };
        assert!(matches!(decode(msg::STATUS, as_bytes(&st)), FeederMessage::Status(s) if s.video_fed == 120 && s.loaded == 1));
        let a = Accept { pts_ns: 5, queue: 2, wait_us: 0 };
        assert!(matches!(decode(msg::ACCEPT, as_bytes(&a)), FeederMessage::Accept(a) if a.queue == 2));
        assert!(matches!(decode(99, &[]), FeederMessage::Unknown { kind: 99, .. }));
    }

    #[test]
    fn hdr10_matches_the_generator_and_the_header_order() {
        let h = Hdr::hdr10(1000, 400);
        assert_eq!((h.display_primaries_x0, h.display_primaries_y0), (13250, 34500));   /* green first: ST 2086 order */
        assert_eq!((h.display_primaries_x2, h.display_primaries_y2), (34000, 16000));   /* red last */
        assert_eq!(h.max_display_mastering_luminance, 10_000_000);
        assert_eq!((h.transfer_characteristics, h.color_primaries, h.matrix_coeffs), (16, 9, 9));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    #[test]
    fn partial_writes_and_interruptions_preserve_all_parts() {
        struct ShortWriter { bytes: Vec<u8>, interrupted: bool }
        impl Write for ShortWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let n = bytes.len().min(3);
                self.bytes.extend_from_slice(&bytes[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut writer = ShortWriter { bytes: Vec::new(), interrupted: false };
        write_all_vectored(&mut writer, &[b"header".as_slice(), b"", b"media", b"payload"]).unwrap();
        assert_eq!(writer.bytes, b"headermediapayload");
        assert_eq!(write_all_vectored(&mut io::sink(), &[b"".as_slice()]).unwrap_err().kind(), io::ErrorKind::WriteZero);
    }

    #[test]
    fn media_and_load_are_byte_exact_without_copying_the_unit() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut client = FeederClient { stream, reader: None };
        let load = Load { codec: codec::H265, width: 3840, height: 2160, fps: 120, audio: audio::OPUS,
            channels: 2, sample_rate: 48000, hdr: 1, hdr_info: Hdr::hdr10(1000, 400), drop_threshold: 2, pts_unit: pts_unit::MS };
        client.send_load(&load).unwrap();
        client.send_media(msg::VIDEO, 123456789012, true, b"access-unit").unwrap();
        let mut bytes = [0u8; 108 + 8 + 16 + 11];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes[..8], &[1, 0, 0, 0, 100, 0, 0, 0]);
        assert_eq!(&bytes[8..108], as_bytes(&load));
        assert_eq!(&bytes[108..116], &[2, 0, 0, 0, 27, 0, 0, 0]);
        assert_eq!(&bytes[116..124], &123456789012i64.to_le_bytes());
        assert_eq!(&bytes[124..132], &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&bytes[132..], b"access-unit");
        let surround = LoadOpus51 { load: Load { channels: 6, ..load }, streams: 4, coupled: 2, mapping: [0, 1, 4, 2, 3, 5] };
        client.send_load_opus51(&surround).unwrap();
        let mut bytes = [0; 116];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes[..8], &[13, 0, 0, 0, 108, 0, 0, 0]);
        assert_eq!(&bytes[8..108], as_bytes(&surround.load));
        assert_eq!(&bytes[108..], &[4, 2, 0, 1, 4, 2, 3, 5]);
        assert!(client.send_media(msg::HDR, 0, false, b"").is_err());
        assert!(client.send(msg::LOAD, &vec![0; MAX_PAYLOAD as usize + 1]).is_err());
    }

    #[test]
    fn reader_delivers_recovery_and_fatal_once_then_closes() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut client = FeederClient { stream, reader: None };
        let (tx, rx) = mpsc::channel();
        client.spawn_reader(move |msg| { tx.send(msg).unwrap(); }).unwrap();
        assert_eq!(client.spawn_reader(|_| {}).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        peer.write_all(&[msg::KEYFRAME, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        peer.write_all(&[msg::ERROR, 0, 0, 0, 9, 0, 0, 0, 2, 0, 0, 0]).unwrap();
        peer.write_all(b"fatal").unwrap();
        peer.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(matches!(rx.recv().unwrap(), Some(FeederMessage::Keyframe)));
        assert!(matches!(rx.recv().unwrap(), Some(FeederMessage::Error { code: 2, text }) if text == "fatal"));
        assert!(rx.recv().unwrap().is_none());
        client.close();
        let mut one = [0];
        assert_eq!(peer.read(&mut one).unwrap(), 0);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn oversized_reply_ends_the_reader_without_allocating_it() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut client = FeederClient { stream, reader: None };
        let (tx, rx) = mpsc::channel();
        client.spawn_reader(move |msg| { tx.send(msg).unwrap(); }).unwrap();
        peer.write_all(as_bytes(&Frame { kind: msg::STATUS, length: MAX_PAYLOAD + 1, ..Default::default() })).unwrap();
        assert!(rx.recv().unwrap().is_none());
        client.close();
    }

    #[test]
    fn connection_rejects_an_incompatible_protocol() {
        let path = std::env::temp_dir().join(format!("lumagame-client-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let hello = Hello { version: PROTO_VERSION + 1, library: 1, pid: 1, window: 0, window_id: [0; 32], library_name: [0; 64] };
            write_all_vectored(&mut peer, &[as_bytes(&Frame { kind: msg::HELLO, length: 112, ..Default::default() }), as_bytes(&hello)]).unwrap();
        });
        let result = FeederClient::connect(&path, Duration::from_secs(1));
        std::fs::remove_file(path).unwrap();
        server.join().unwrap();
        assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::InvalidData));
    }
}
