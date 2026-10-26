//! The engine's end of the feeder socket: gfn/feeder/src/gf_proto.h, byte for byte (`#[repr(C)]` on a little-endian LP64 target; the
//! tests check the sizes the C side prints with `--probe`). Frames are written on the caller's thread with one `writev`; what the
//! feeder sends back is read on a thread of its own and handed to a closure.
use std::io::{self, IoSlice, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const PROTO_VERSION: u32 = 1;
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
    pub const LOADED: u8 = 16;
    pub const EVENT: u8 = 17;
    pub const ACCEPT: u8 = 18;
    pub const DROP: u8 = 19;
    pub const STATUS: u8 = 20;
    pub const ERROR: u8 = 21;
    pub const PONG: u8 = 22;
    pub const HELLO: u8 = 23;
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

/// One `writev` per frame when the socket takes it whole, the rest written piecewise: no copy of the access unit.
fn write_all_vectored(stream: &mut UnixStream, parts: &[&[u8]]) -> io::Result<()> {
    let total: usize = parts.iter().map(|p| p.len()).sum();
    let slices: Vec<IoSlice<'_>> = parts.iter().map(|p| IoSlice::new(p)).collect();
    let mut done = stream.write_vectored(&slices)?;
    while done < total {
        let mut skip = done;
        for part in parts {
            if skip >= part.len() {
                skip -= part.len();
                continue;
            }
            let n = stream.write(&part[skip..])?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "the feeder socket took nothing"));
            }
            done += n;
            break;
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
    Hello { library: bool, pid: u32, own_window: bool, window_id: String, library_name: String },
    Loaded { ok: bool, text: String },
    Event { kind: i32, num: i64, text: String },
    Accept(Accept),
    Drop(Drop),
    Status(Status),
    Error { code: i32, text: String },
    Pong { echo: i64, feeder_ns: i64 },
    Unknown { kind: u8, length: u32 },
}

pub fn decode(kind: u8, p: &[u8]) -> FeederMessage {
    match kind {
        msg::HELLO => match read_struct::<Hello>(p) {
            Some(h) => FeederMessage::Hello {
                library: h.library != 0,
                pid: h.pid,
                own_window: h.window != 0,
                window_id: c_string(&h.window_id),
                library_name: c_string(&h.library_name),
            },
            None => FeederMessage::Unknown { kind, length: p.len() as u32 },
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
                Err(e) if Instant::now() < end => {
                    let _ = e;
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(io::Error::new(e.kind(), format!("feeder socket {}: {e}", path.display()))),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut s = Self { stream, reader: None };
        let (kind, payload) = s.read_frame()?;
        let hello = decode(kind, &payload);
        if !matches!(hello, FeederMessage::Hello { .. }) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("the feeder opened with message {kind}, not HELLO")));
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
        let mut stream = self.stream.try_clone()?;
        self.reader = Some(
            thread::Builder::new().name("gfn-feeder-reader".to_owned()).spawn(move || {
                let mut head = [0u8; 8];
                loop {
                    if stream.read_exact(&mut head).is_err() {
                        break;
                    }
                    let f: Frame = read_struct(&head).unwrap();
                    if f.length > MAX_PAYLOAD {
                        break;
                    }
                    let mut payload = vec![0u8; f.length as usize];
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
        let f = Frame { kind, flags: 0, reserved: 0, length: payload.len() as u32 };
        write_all_vectored(&mut self.stream, &[as_bytes(&f), payload])
    }

    pub fn send_media(&mut self, kind: u8, pts_ns: i64, key: bool, data: &[u8]) -> io::Result<()> {
        if data.len() as u32 > MAX_PAYLOAD - 16 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "an access unit larger than the feeder takes"));
        }
        let f = Frame { kind, flags: 0, reserved: 0, length: (16 + data.len()) as u32 };
        let h = MediaHead { pts_ns, key: u8::from(key), pad: [0; 7] };
        write_all_vectored(&mut self.stream, &[as_bytes(&f), as_bytes(&h), data])
    }

    pub fn send_load(&mut self, load: &Load) -> io::Result<()> {
        self.send(msg::LOAD, as_bytes(load))
    }

    pub fn send_hdr(&mut self, hdr: &Hdr) -> io::Result<()> {
        self.send(msg::HDR, as_bytes(hdr))
    }

    pub fn send_state(&mut self, foreground: bool) -> io::Result<()> {
        self.send(msg::STATE, &[u8::from(foreground)])
    }

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
    pub fn close(mut self) {
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
        let mut h = Hello { version: 1, library: 1, pid: 7, window: 0, window_id: [0; 32], library_name: [0; 64] };
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
