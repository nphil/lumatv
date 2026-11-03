//! The frame endpoint: how a process other than the app puts frames it did not draw (a camera's pictures; later a decoder's) on a
//! layer of the screen without anything being copied.
//!
//! A producer connects to [`SOCKET_PATH`], says `Hello`, and `Attach`es a *source*: a few buffers of one size (dma-bufs, the
//! descriptors travelling with the message) that it fills. The app draws a source with a `Content::Live` layer whose `source`
//! names it, and decides where the layer is, how it moves and what is over it, as for any layer; the pictures never pass through
//! the app. Each picture the producer finishes is one small message naming the buffer it is in (`Frame`); the compositor
//! puts the buffers on the GPU once, between frames, after `Attach` (and says `Attached` when the last is there, so that no frame
//! is the one that pays for the import), and draws from the newest one. Nothing is read or written by the CPU
//! and no pixel is uploaded for a frame: that is what this endpoint is for, and what the compositor's tests pin.
//!
//! **Who may write a buffer.** After `Attach` every buffer is the producer's. A buffer named by a `Frame` is the compositor's
//! until `Released` says it is the producer's again; the compositor releases a buffer when a later `Frame`, a `Clear` or a
//! `Detach` replaces it (and the frame that last drew it has long been over: the compositor draws one frame at a time). The
//! producer never writes a buffer between its `Frame` and its `Released`, and a source whose producer is gone (the connection
//! ended) is detached.
//!
//! **Privacy.** The compositor draws what it is given and knows nothing of lens covers: a producer that learns its pictures must
//! not be shown says `Clear`, and the layer shows nothing from that moment until the next `Frame`.
//!
//! It is the same framing as the app's protocol ([`write_frame`], [`read_frame`]) on a Unix stream socket, with the descriptors
//! as ancillary data (see [`crate::fds`]). Only the sockets' owners and the `graphics` group can connect (`/run/lumashow`).

use crate::fds::{Fill, MAX_FDS, Reader, send_fds};
use crate::{SourceId, write_frame};
use serde::{Deserialize, Serialize};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Where the compositor listens for producers.
pub const SOCKET_PATH: &str = "/run/lumashow/frames.sock";

/// Bumped whenever a message changes shape. The compositor refuses a `Hello` of another version and answers nothing else until
/// it has had one of its own.
pub const PROTOCOL_VERSION: u32 = 1;

/// The most buffers a source has: the camera service makes three.
pub const MAX_BUFFERS: usize = 4;
const _: () = assert!(MAX_BUFFERS <= MAX_FDS);

/// How the pixels of a buffer are laid out in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    /// Red, green, blue, alpha: four bytes a pixel, premultiplied.
    Rgba8888,
}

impl Format {
    /// Bytes a pixel takes in memory.
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Rgba8888 => 4,
        }
    }
}

/// What every buffer of a source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub width: u32,
    pub height: u32,
    /// Bytes from the start of one row to the next (at least `width` pixels' worth).
    pub stride: u32,
    pub format: Format,
}

impl Spec {
    /// Bytes of picture a buffer must hold: the rows, the last without padding after it.
    pub fn bytes(&self) -> u64 {
        u64::from(self.stride) * u64::from(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// First message on a connection. Answered with [`Event::Welcome`].
    Hello { version: u32 },
    /// Makes (or, for a source this connection already has, replaces) the source with `buffers` buffers of `spec`, the
    /// descriptors of which are carried with this message, in order: buffer 0 first. Nothing is shown until the first `Frame`.
    /// Answered with [`Event::Attached`] once the GPU has them, or [`Event::Refused`].
    Attach {
        source: SourceId,
        spec: Spec,
        buffers: u8,
    },
    /// A picture is complete in `buffer`, which is the compositor's until [`Event::Released`].
    Frame { source: SourceId, buffer: u8 },
    /// The layers that show the source show nothing from now until the next `Frame`; the buffer they showed is released.
    Clear { source: SourceId },
    /// Ends the source: its layers show nothing, every buffer is released and the GPU lets go of them.
    Detach { source: SourceId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    Welcome {
        version: u32,
    },
    /// The source's buffers are on the GPU: `Frame`s may follow.
    Attached {
        source: SourceId,
    },
    /// The compositor will not read `buffer` again until a `Frame` names it: the producer may write it.
    Released {
        source: SourceId,
        buffer: u8,
    },
    /// The source is over (the producer detached it, or another `Attach` replaced it).
    Detached {
        source: SourceId,
    },
    /// A request was refused, in words for the producer's author. The connection stays usable unless the message says
    /// otherwise by ending it.
    Refused {
        source: Option<SourceId>,
        message: String,
    },
}

/// The producer's end of the socket: what the camera hub (and, later, a decoder) holds. Reads never block; a write that cannot
/// be made at once is an error, because a compositor that has stopped reading is a connection to end, not to wait for.
pub struct Producer {
    stream: UnixStream,
    reader: Reader,
    /// How the connection ended, once what was said before it has been handed over.
    ended: Option<(io::ErrorKind, String)>,
}

impl Producer {
    /// Connects to the endpoint at `path` and says hello, waiting up to `wait` for the welcome.
    pub fn connect(path: &Path, wait: Duration) -> io::Result<Self> {
        let mut producer = Self::from_stream(UnixStream::connect(path)?)?;
        producer.hello()?;
        let until = std::time::Instant::now() + wait;
        loop {
            for event in producer.events()? {
                match event {
                    Event::Welcome { .. } => return Ok(producer),
                    Event::Refused { message, .. } => {
                        return Err(io::Error::new(io::ErrorKind::ConnectionRefused, message));
                    }
                    _ => {}
                }
            }
            if std::time::Instant::now() >= until {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the compositor did not welcome the producer",
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A producer over a connection that is made already (nothing is said yet: [`Producer::hello`]).
    pub fn from_stream(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            reader: Reader::default(),
            ended: None,
        })
    }

    /// Says hello; the welcome comes back as an event.
    pub fn hello(&mut self) -> io::Result<()> {
        self.send_request(
            &Request::Hello {
                version: PROTOCOL_VERSION,
            },
            &[],
        )
    }

    /// Sends any request with the descriptors given: what the typed calls below are made of, and the way to say what they
    /// never would (the tests of a compositor do).
    pub fn send_request(&mut self, request: &Request, fds: &[BorrowedFd<'_>]) -> io::Result<()> {
        let raw: Vec<RawFd> = fds.iter().map(AsRawFd::as_raw_fd).collect();
        self.send(request, &raw)
    }

    fn send(&mut self, request: &Request, fds: &[RawFd]) -> io::Result<()> {
        let mut frame = Vec::new();
        write_frame(&mut frame, request)?;
        send_fds(self.stream.as_raw_fd(), &frame, fds)
    }

    /// Sends [`Request::Attach`] with the buffers' descriptors (borrowed: the compositor gets duplicates).
    pub fn attach(
        &mut self,
        source: SourceId,
        spec: Spec,
        buffers: &[BorrowedFd<'_>],
    ) -> io::Result<()> {
        let raw: Vec<RawFd> = buffers.iter().map(AsRawFd::as_raw_fd).collect();
        let count = u8::try_from(raw.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many buffers"))?;
        self.send(
            &Request::Attach {
                source,
                spec,
                buffers: count,
            },
            &raw,
        )
    }

    pub fn frame(&mut self, source: SourceId, buffer: u8) -> io::Result<()> {
        self.send(&Request::Frame { source, buffer }, &[])
    }

    pub fn clear(&mut self, source: SourceId) -> io::Result<()> {
        self.send(&Request::Clear { source }, &[])
    }

    pub fn detach(&mut self, source: SourceId) -> io::Result<()> {
        self.send(&Request::Detach { source }, &[])
    }

    /// Everything the compositor has said that has not been taken. An error is the end of the connection (the compositor went
    /// away, or said something that is not a message): what it said last (a refusal that ends the connection) is handed over
    /// first, and the end is the next call's error.
    pub fn events(&mut self) -> io::Result<Vec<Event>> {
        if let Some((kind, message)) = &self.ended {
            return Err(io::Error::new(*kind, message.clone()));
        }
        let mut events = Vec::new();
        loop {
            let step = self.reader.fill(self.stream.as_raw_fd()).and_then(|fill| {
                while let Some(event) = self.reader.message::<Event>()? {
                    events.push(event);
                }
                Ok(fill)
            });
            let end = match step {
                Ok(Fill::Data) => continue,
                Ok(Fill::Empty) => return Ok(events),
                Ok(Fill::Closed) => io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "the compositor closed the frame endpoint",
                ),
                Err(e) => e,
            };
            self.ended = Some((end.kind(), end.to_string()));
            return if events.is_empty() {
                Err(end)
            } else {
                Ok(events)
            };
        }
    }

    /// The socket, for a poll that waits for the compositor to speak.
    pub fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_FRAME, read_frame};
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::{FromRawFd, OwnedFd};

    fn tempfile() -> File {
        let mut template = *b"/tmp/lumashow-frames-XXXXXX\0";
        // SAFETY: a writable NUL-terminated `mkstemp` template.
        let fd = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
        assert!(fd >= 0, "mkstemp: {}", io::Error::last_os_error());
        // SAFETY: the file is unlinked at once and its descriptor is ours alone.
        unsafe {
            libc::unlink(template.as_ptr().cast());
            File::from_raw_fd(fd)
        }
    }

    fn file_with(text: &str) -> File {
        let mut file = tempfile();
        file.write_all(text.as_bytes()).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file
    }

    fn read_all(fd: OwnedFd) -> String {
        let mut text = String::new();
        File::from(fd).read_to_string(&mut text).unwrap();
        text
    }

    const SPEC: Spec = Spec {
        width: 400,
        height: 300,
        stride: 1600,
        format: Format::Rgba8888,
    };

    #[test]
    fn every_message_survives_the_wire() {
        let requests = [
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::Attach {
                source: SourceId(7),
                spec: SPEC,
                buffers: 3,
            },
            Request::Frame {
                source: SourceId(7),
                buffer: 2,
            },
            Request::Clear {
                source: SourceId(7),
            },
            Request::Detach {
                source: SourceId(7),
            },
        ];
        let events = [
            Event::Welcome {
                version: PROTOCOL_VERSION,
            },
            Event::Attached {
                source: SourceId(7),
            },
            Event::Released {
                source: SourceId(7),
                buffer: 1,
            },
            Event::Detached {
                source: SourceId(7),
            },
            Event::Refused {
                source: None,
                message: "no".into(),
            },
        ];
        let mut wire = Vec::new();
        for r in &requests {
            write_frame(&mut wire, r).unwrap();
        }
        for e in &events {
            write_frame(&mut wire, e).unwrap();
        }
        let mut input = wire.as_slice();
        let mut buf = Vec::new();
        for r in &requests {
            assert_eq!(
                &read_frame::<Request>(&mut input, &mut buf)
                    .unwrap()
                    .unwrap(),
                r
            );
        }
        for e in &events {
            assert_eq!(
                &read_frame::<Event>(&mut input, &mut buf).unwrap().unwrap(),
                e
            );
        }
        // A frame message is small: it is sent for every picture of a stream.
        let mut one = Vec::new();
        write_frame(&mut one, &requests[2]).unwrap();
        assert!(one.len() <= 24, "{} bytes", one.len());
    }

    #[test]
    fn a_source_arrives_with_its_buffers_in_order_and_each_is_its_own_descriptor() {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        let mut producer = Producer::from_stream(a).unwrap();
        let files = [file_with("zero"), file_with("one"), file_with("two")];
        let borrowed: Vec<BorrowedFd<'_>> = files.iter().map(std::os::fd::AsFd::as_fd).collect();
        producer.attach(SourceId(1), SPEC, &borrowed).unwrap();
        producer.frame(SourceId(1), 2).unwrap();
        let mut reader = Reader::default();
        assert_eq!(reader.fill(b.as_raw_fd()).unwrap(), Fill::Data);
        let Some(Request::Attach { buffers, spec, .. }) = reader.message().unwrap() else {
            panic!("the attach came first");
        };
        assert_eq!((buffers, spec), (3, SPEC));
        let fds = reader.take_fds(usize::from(buffers)).unwrap();
        let texts: Vec<String> = fds.into_iter().map(read_all).collect();
        assert_eq!(texts, ["zero", "one", "two"]);
        // Data that follows a message with descriptors is a read of its own: a read never reaches across the message that
        // carried them.
        assert_eq!(reader.fill(b.as_raw_fd()).unwrap(), Fill::Data);
        assert!(matches!(
            reader.message().unwrap(),
            Some(Request::Frame { buffer: 2, .. })
        ));
        assert_eq!(reader.waiting_fds(), 0);
    }

    #[test]
    fn a_message_that_comes_a_few_bytes_at_a_time_is_one_message_and_its_descriptors_are_not_lost()
    {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        let file = file_with("pixels");
        let mut frame = Vec::new();
        write_frame(
            &mut frame,
            &Request::Attach {
                source: SourceId(9),
                spec: SPEC,
                buffers: 1,
            },
        )
        .unwrap();
        let mut reader = Reader::default();
        // The first byte alone carries the descriptor; the rest follows in pieces.
        send_fds(a.as_raw_fd(), &frame[..1], &[file.as_raw_fd()]).unwrap();
        assert_eq!(reader.fill(b.as_raw_fd()).unwrap(), Fill::Data);
        assert!(reader.message::<Request>().unwrap().is_none());
        for piece in frame[1..].chunks(3) {
            send_fds(a.as_raw_fd(), piece, &[]).unwrap();
            assert_eq!(reader.fill(b.as_raw_fd()).unwrap(), Fill::Data);
        }
        assert!(matches!(
            reader.message::<Request>().unwrap(),
            Some(Request::Attach { buffers: 1, .. })
        ));
        assert_eq!(read_all(reader.take_fds(1).unwrap().remove(0)), "pixels");
    }

    #[test]
    fn descriptors_nobody_asks_for_are_the_readers_to_close_and_too_many_end_the_connection() {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        let file = tempfile();
        let mut reader = Reader::default();
        let mut frame = Vec::new();
        write_frame(
            &mut frame,
            &Request::Hello {
                version: PROTOCOL_VERSION,
            },
        )
        .unwrap();
        assert!(reader.take_fds(1).is_none());
        let mut ended = false;
        for _ in 0..8 {
            send_fds(a.as_raw_fd(), &frame, &[file.as_raw_fd(); 4]).unwrap();
            match reader.fill(b.as_raw_fd()) {
                Ok(_) => {
                    let _ = reader.message::<Request>();
                }
                Err(e) => {
                    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
                    ended = true;
                    break;
                }
            }
        }
        assert!(ended, "descriptors that are never claimed fill up");
        reader.drop_fds();
        assert_eq!(reader.waiting_fds(), 0);
    }

    #[test]
    fn a_message_may_carry_at_most_the_limit() {
        let (a, _b) = UnixStream::pair().unwrap();
        let file = tempfile();
        let e = send_fds(a.as_raw_fd(), b"x", &[file.as_raw_fd(); MAX_FDS + 1]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn an_oversize_frame_and_one_that_does_not_decode_are_errors_and_the_next_frame_is_read() {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        let mut reader = Reader::default();
        // A body of one byte that is not a request.
        send_fds(a.as_raw_fd(), &[1, 0, 0, 0, 0xff], &[]).unwrap();
        let mut good = Vec::new();
        write_frame(
            &mut good,
            &Request::Clear {
                source: SourceId(3),
            },
        )
        .unwrap();
        send_fds(a.as_raw_fd(), &good, &[]).unwrap();
        reader.fill(b.as_raw_fd()).unwrap();
        assert!(reader.message::<Request>().is_err());
        assert!(matches!(
            reader.message::<Request>().unwrap(),
            Some(Request::Clear { .. })
        ));
        send_fds(a.as_raw_fd(), &(MAX_FRAME + 1).to_le_bytes(), &[]).unwrap();
        reader.fill(b.as_raw_fd()).unwrap();
        assert_eq!(
            reader.message::<Request>().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn the_producer_hears_every_event_the_compositor_has_sent_and_the_end_of_the_connection() {
        let (a, mut b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let mut producer = Producer::from_stream(a).unwrap();
        assert!(producer.events().unwrap().is_empty());
        write_frame(
            &mut b,
            &Event::Released {
                source: SourceId(1),
                buffer: 0,
            },
        )
        .unwrap();
        write_frame(
            &mut b,
            &Event::Released {
                source: SourceId(1),
                buffer: 1,
            },
        )
        .unwrap();
        assert_eq!(producer.events().unwrap().len(), 2);
        drop(b);
        assert_eq!(
            producer.events().unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
    }
}
