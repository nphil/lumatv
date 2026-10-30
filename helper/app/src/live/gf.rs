//! The wire to `lumatv-gfn-feeder` (`gfn/feeder/src/gf_proto.h`): a Unix stream socket of `gf_frame` headers and little-endian structs.
//! The feeder is the 32-bit process that owns the video window and LG's NDL DirectMedia pipeline; this side loads it with a codec and a
//! size and sends whole access units with their presentation times. The structs are `#[repr(C)]` mirrors of the header's; the sizes
//! are asserted so a field added on one side cannot pass unnoticed.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub const PROTO_VERSION: u32 = 1;
pub const MAX_PAYLOAD: usize = 8 << 20;

pub const LOAD: u8 = 1;
pub const VIDEO: u8 = 2;
pub const FLUSH: u8 = 4;
pub const UNLOAD: u8 = 5;
pub const STATE: u8 = 6;
pub const PING: u8 = 8;
pub const APP: u8 = 9;
pub const TEXTURE: u8 = 10;
pub const PIPE: u8 = 11;
pub const SINK: u8 = 12;
pub const LOADED: u8 = 16;
pub const EVENT: u8 = 17;
pub const ACCEPT: u8 = 18;
pub const DROP: u8 = 19;
pub const STATUS: u8 = 20;
pub const ERROR: u8 = 21;
pub const PONG: u8 = 22;
pub const HELLO: u8 = 23;

pub const CODEC_H264: u32 = 1;
pub const CODEC_H265: u32 = 2;
pub const PTS_NS: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub kind: u8,
    pub flags: u8,
    pub reserved: u16,
    pub length: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Hello {
    pub version: u32,
    pub library: u32,
    pub pid: u32,
    pub window: u32,
    pub window_id: [u8; 32],
    pub library_name: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Hdr {
    pub primaries: [u32; 6],
    pub white: [u32; 2],
    pub luminance: [u32; 2],
    pub light: [u32; 2],
    pub transfer: u32,
    pub colour_primaries: u32,
    pub matrix: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
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

const _: () = {
    assert!(std::mem::size_of::<Frame>() == 8);
    assert!(std::mem::size_of::<Hello>() == 112);
    assert!(std::mem::size_of::<Load>() == 100);
    assert!(std::mem::size_of::<MediaHead>() == 16);
    assert!(std::mem::size_of::<Accept>() == 16);
    assert!(std::mem::size_of::<Drop>() == 16);
    assert!(std::mem::size_of::<Status>() == 104);
};

fn bytes_of<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: the structs above are plain `repr(C)` numbers with no padding bytes read as anything but bytes of the struct.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

fn read_struct<T: Copy>(b: &[u8]) -> Option<T> {
    (b.len() >= std::mem::size_of::<T>()).then(|| {
        // SAFETY: the length is checked; an unaligned read of a plain `repr(C)` struct of numbers.
        unsafe { std::ptr::read_unaligned(b.as_ptr().cast::<T>()) }
    })
}

fn c_string(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// What the feeder says.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Hello {
        library: bool,
        pid: u32,
        own_window: bool,
        window_id: String,
        library_name: String,
    },
    Loaded {
        ok: bool,
        text: String,
    },
    Event {
        kind: i32,
        num: i64,
        text: String,
    },
    Accept(Accept2),
    Drop {
        pts_ns: i64,
        reason: i32,
    },
    Status(Status2),
    Error {
        code: i32,
        text: String,
    },
    Pong {
        echo: i64,
        clock_ns: i64,
    },
    Other(u8),
}

/// `Accept` as a plain comparable value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accept2 {
    pub pts_ns: i64,
    pub queue: i32,
    pub wait_us: i32,
}

/// The parts of `Status` the live path reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status2 {
    pub video_fed: u64,
    pub video_dropped: u64,
    pub queue_p99: u32,
    pub buffer_full: u32,
    pub loaded: bool,
    pub playing: bool,
    pub rss_kb: u32,
}

pub fn decode(kind: u8, p: &[u8]) -> Message {
    match kind {
        HELLO => match read_struct::<Hello>(p) {
            Some(h) => Message::Hello {
                library: h.library == 1,
                pid: h.pid,
                own_window: h.window == 1,
                window_id: c_string(&h.window_id),
                library_name: c_string(&h.library_name),
            },
            None => Message::Other(kind),
        },
        LOADED => Message::Loaded {
            ok: read_struct::<i32>(p) == Some(1),
            text: c_string(p.get(4..).unwrap_or(&[])),
        },
        EVENT => {
            let (t, n) = (
                read_struct::<i32>(p).unwrap_or(0),
                read_struct::<i64>(p.get(8..).unwrap_or(&[])).unwrap_or(0),
            );
            Message::Event {
                kind: t,
                num: n,
                text: c_string(p.get(16..).unwrap_or(&[])),
            }
        }
        ACCEPT => match read_struct::<Accept>(p) {
            Some(a) => Message::Accept(Accept2 {
                pts_ns: a.pts_ns,
                queue: a.queue,
                wait_us: a.wait_us,
            }),
            None => Message::Other(kind),
        },
        DROP => match read_struct::<Drop>(p) {
            Some(d) => Message::Drop {
                pts_ns: d.pts_ns,
                reason: d.reason,
            },
            None => Message::Other(kind),
        },
        STATUS => match read_struct::<Status>(p) {
            Some(s) => Message::Status(Status2 {
                video_fed: s.video_fed,
                video_dropped: s.video_dropped,
                queue_p99: s.queue_p99,
                buffer_full: s.buffer_full,
                loaded: s.loaded == 1,
                playing: s.playing == 1,
                rss_kb: s.rss_kb,
            }),
            None => Message::Other(kind),
        },
        ERROR => Message::Error {
            code: read_struct::<i32>(p).unwrap_or(0),
            text: c_string(p.get(4..).unwrap_or(&[])),
        },
        PONG => Message::Pong {
            echo: read_struct::<i64>(p).unwrap_or(0),
            clock_ns: read_struct::<i64>(p.get(8..).unwrap_or(&[])).unwrap_or(0),
        },
        other => Message::Other(other),
    }
}

/// The socket to one feeder.
pub struct Client {
    stream: UnixStream,
}

impl Client {
    /// Connects and reads the feeder's `hello` (the first thing it says).
    pub fn connect(path: &Path, timeout: Duration) -> io::Result<(Self, Message)> {
        let deadline = std::time::Instant::now() + timeout;
        let stream = loop {
            match UnixStream::connect(path) {
                Ok(s) => break s,
                Err(e) if std::time::Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e),
            }
        };
        stream.set_read_timeout(Some(timeout))?;
        let mut me = Self { stream };
        let (kind, payload) = me.read_frame()?;
        let hello = decode(kind, &payload);
        if !matches!(hello, Message::Hello { .. }) {
            return Err(io::Error::other(format!(
                "the feeder did not say hello: {hello:?}"
            )));
        }
        Ok((me, hello))
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }

    pub fn try_clone(&self) -> io::Result<UnixStream> {
        self.stream.try_clone()
    }

    /// Closes the connection (the feeder, started with `--once`, ends when it is gone).
    pub fn close(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    pub fn read_frame(&mut self) -> io::Result<(u8, Vec<u8>)> {
        read_frame(&mut self.stream)
    }

    pub fn send(&mut self, kind: u8, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_PAYLOAD {
            return Err(io::Error::other("a payload over the feeder's limit"));
        }
        let head = Frame {
            kind,
            flags: 0,
            reserved: 0,
            length: payload.len() as u32,
        };
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(bytes_of(&head));
        buf.extend_from_slice(payload);
        self.stream.write_all(&buf)
    }

    pub fn send_load(&mut self, codec: u32, width: u32, height: u32, fps: u32, drop_threshold: i32) -> io::Result<()> {
        let load = Load {
            codec,
            width,
            height,
            fps,
            audio: 0,
            channels: 0,
            sample_rate: 0,
            hdr: 0,
            hdr_info: Hdr::default(),
            drop_threshold,
            pts_unit: PTS_NS,
        };
        self.send(LOAD, bytes_of(&load))
    }

    pub fn send_video(&mut self, pts_ns: i64, key: bool, data: &[u8]) -> io::Result<()> {
        let head = MediaHead {
            pts_ns,
            key: u8::from(key),
            pad: [0; 7],
        };
        let mut buf = Vec::with_capacity(16 + data.len());
        buf.extend_from_slice(bytes_of(&head));
        buf.extend_from_slice(data);
        self.send(VIDEO, &buf)
    }

    pub fn send_state(&mut self, foreground: bool) -> io::Result<()> {
        self.send(STATE, &[u8::from(foreground)])
    }

    pub fn send_flush(&mut self) -> io::Result<()> {
        self.send(FLUSH, &[])
    }

    pub fn send_unload(&mut self) -> io::Result<()> {
        self.send(UNLOAD, &[])
    }

    /// The app id NDL is opened for from the next load on; empty is the window's own (`GF_APP`).
    pub fn send_app(&mut self, app: &str) -> io::Result<()> {
        self.send(APP, app.as_bytes())
    }

    /// The library the next load runs in (`GF_PIPE`): `ndl`, the cloud game's, or `smp`, the starfish pipeline an app's own video has, the one that can take the
    /// sub window (`gfn/feeder/src/gf_pipe.h`).
    pub fn send_pipe(&mut self, pipe: &str) -> io::Result<()> {
        self.send(PIPE, pipe.as_bytes())
    }

    /// The video sink the next load asks for (`GF_SINK`): empty for the main window as every pipeline gets, `sub_video` for the sub window beside an app's
    /// video (the starfish pipeline only).
    pub fn send_sink(&mut self, sink: &str) -> io::Result<()> {
        self.send(SINK, sink.as_bytes())
    }

    /// The picture of the next load is decoded in the feeder and painted into its window's buffers at `width` x `height` (`GF_TEXTURE`); 0 x 0 is the
    /// pipeline again. The compositor stretches the buffer over the claim's rectangle, so the size is the picture's own, not the box's.
    pub fn send_texture(&mut self, width: u32, height: u32) -> io::Result<()> {
        let mut payload = [0u8; 8];
        payload[..4].copy_from_slice(&width.to_le_bytes());
        payload[4..].copy_from_slice(&height.to_le_bytes());
        self.send(TEXTURE, &payload)
    }

    pub fn send_ping(&mut self, echo: i64) -> io::Result<()> {
        self.send(PING, &echo.to_le_bytes())
    }
}

/// One frame from any reader of the socket (the session's reader thread uses a clone).
pub fn read_frame(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 8];
    stream.read_exact(&mut head)?;
    let frame: Frame = read_struct(&head).expect("eight bytes");
    let length = frame.length as usize;
    if length > MAX_PAYLOAD {
        return Err(io::Error::other("a frame over the feeder's limit"));
    }
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload)?;
    Ok((frame.kind, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_the_structs_match_the_header() {
        let mut hello = Hello {
            version: 1,
            library: 1,
            pid: 7,
            window: 1,
            window_id: [0; 32],
            library_name: [0; 64],
        };
        hello.window_id[.."_Window_Id_9".len()].copy_from_slice(b"_Window_Id_9");
        hello.library_name[..3].copy_from_slice(b"ndl");
        let m = decode(HELLO, bytes_of(&hello));
        assert_eq!(
            m,
            Message::Hello {
                library: true,
                pid: 7,
                own_window: true,
                window_id: "_Window_Id_9".into(),
                library_name: "ndl".into()
            }
        );
        let status = Status {
            video_fed: 60,
            loaded: 1,
            playing: 1,
            ..Status::default()
        };
        assert_eq!(
            decode(STATUS, bytes_of(&status)),
            Message::Status(Status2 {
                video_fed: 60,
                loaded: true,
                playing: true,
                ..Status2::default()
            })
        );
        let mut loaded = 1i32.to_le_bytes().to_vec();
        loaded.extend_from_slice(b"playing\0");
        assert_eq!(
            decode(LOADED, &loaded),
            Message::Loaded {
                ok: true,
                text: "playing".into()
            }
        );
        assert_eq!(decode(99, &[]), Message::Other(99));
        // a frame written by the client reads back the same
        let (a, mut b) = UnixStream::pair().unwrap();
        let mut client = Client { stream: a };
        client
            .send_video(1_000_000_000, true, &[0, 0, 0, 1, 0x65])
            .unwrap();
        let (kind, payload) = read_frame(&mut b).unwrap();
        assert_eq!(kind, VIDEO);
        let head: MediaHead = read_struct(&payload).unwrap();
        assert_eq!((head.pts_ns, head.key), (1_000_000_000, 1));
        assert_eq!(&payload[16..], &[0, 0, 0, 1, 0x65]);
        client.send_load(CODEC_H264, 1280, 720, 15, -1).unwrap();
        let (kind, payload) = read_frame(&mut b).unwrap();
        assert_eq!((kind, payload.len()), (LOAD, 100));
        let load: Load = read_struct(&payload).unwrap();
        assert_eq!(
            (
                load.codec,
                load.width,
                load.height,
                load.fps,
                load.pts_unit,
                load.drop_threshold
            ),
            (1, 1280, 720, 15, PTS_NS, -1)
        );
    }
}
