//! The contract between the compositor and a display service, which puts the compositor's planes on the panel's hardware
//! layers for it.
//!
//! The compositor renders each plane of the picture into buffers of its own and hands the service the buffers' dma-bufs
//! once; from then on a frame is the message "these planes show these buffers from the next refresh on". The service is
//! the device's half of scan-out: what its hardware layers are, how they are put on the panel and how a frame is known to
//! be there stay behind this contract, so a device with other hardware writes its own service (or a route that speaks to
//! its planes directly) and the compositor does not change.
//!
//! It is the same framing as the app's protocol ([`write_frame`], [`read_frame`]) on a Unix stream socket, with the
//! dma-bufs carried as ancillary data by [`Channel`]. It is strictly lock-step: every request gets exactly one event
//! back, so neither side ever has to match replies to requests.

use crate::fds::{recv_fds, send_fds};
use crate::{read_frame, write_frame};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Where the service listens.
pub const SOCKET_PATH: &str = "/run/lumashow/display.sock";

/// Bumped whenever a message changes shape. The service refuses a `Hello` with another version, and answers nothing else
/// until it has had one with its own.
pub const PROTOCOL_VERSION: u32 = 2;

/// How the pixels of a plane's buffer are laid out in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    /// Blue, green, red, alpha: four bytes a pixel.
    Bgra8888,
    /// Red, green, blue, alpha: four bytes a pixel.
    Rgba8888,
    /// Packed 4:2:2 YUV, two bytes a pixel: Y0 U Y1 V.
    Yuyv,
    /// Packed 4:2:2 YUV, two bytes a pixel: U Y0 V Y1.
    Uyvy,
}

impl PixelFormat {
    /// Bytes a pixel takes in memory.
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Bgra8888 | Self::Rgba8888 => 4,
            Self::Yuyv | Self::Uyvy => 2,
        }
    }

    /// Whether the pixels are YUV, and so need a [`ColorMatrix`].
    pub const fn is_yuv(self) -> bool {
        matches!(self, Self::Yuyv | Self::Uyvy)
    }
}

/// How a plane's pixels combine with the planes below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blend {
    /// Replaces what is below; the pixels' alpha is ignored.
    Opaque,
    /// Premultiplied alpha: the plane's pixels are added to what is below scaled by one minus their alpha.
    Premultiplied,
}

/// How a YUV picture is turned into RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMatrix {
    /// BT.601, luma 0 to 255.
    Bt601Full,
    /// BT.601, luma 16 to 235.
    Bt601Limited,
    /// BT.709, luma 16 to 235.
    Bt709Limited,
}

/// What a display service offers, told when the compositor says `Hello`. The compositor plans its planes from it, and a
/// service that offers too little for a route is a service that route is not used with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// The panel, in pixels.
    pub width: u32,
    pub height: u32,
    /// How many planes a session may have, bottom to top.
    pub planes: u8,
    /// The layouts a plane's buffer may have, most wanted first.
    pub formats: Vec<PixelFormat>,
    /// The ways a plane may combine with the ones below it.
    pub blends: Vec<Blend>,
    /// The matrices the hardware converts YUV with (empty: it shows no YUV plane).
    pub matrices: Vec<ColorMatrix>,
    /// Whether it can show a plane at another size than its buffer's. A plane is never asked to be.
    pub scaling: bool,
    /// Whether a plane may be smaller than the panel, put anywhere on it and faded (`Show::at`, `Show::alpha`); without it
    /// every plane is panel-sized, at the corner, fully opaque.
    pub placement: bool,
}

/// One plane of a session: what its buffers are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneSpec {
    pub format: PixelFormat,
    pub blend: Blend,
    /// The size of its buffers, which is the size it is shown at: no plane is scaled.
    pub width: u32,
    pub height: u32,
    /// How a YUV plane's pixels become RGB; `None` for the RGB layouts.
    pub matrix: Option<ColorMatrix>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// First message. Nothing is touched: answered with [`Event::Offer`].
    Hello { version: u32 },
    /// Starts a session. The panel is put back as it was before any plane (and confirmed to be), every buffer of an earlier
    /// session is forgotten, and `planes`, bottom to top, are what this session will show. Answered with [`Event::Done`],
    /// or [`Event::Failed`] for what the offer does not allow.
    Begin { planes: Vec<PlaneSpec> },
    /// A buffer of plane `plane`, the dma-buf carried with this message: `stride` bytes to a row, `len` bytes in all,
    /// in panel orientation. Answered with [`Event::Done`].
    Buffer {
        plane: u8,
        index: u8,
        stride: u32,
        len: u32,
    },
    /// The frame: the buffer each of `show`'s planes has from the next refresh on, all together. Planes that are not
    /// listed keep what they show; a frame that lists none only waits for the refresh. Answered with
    /// [`Event::Applied`] once the panel is showing it.
    Present { frame: u64, show: Vec<Show> },
    /// Asks whether the layers on the panel are what the service put there. Answered with [`Event::Checked`].
    Check,
    /// Puts the panel back as it was before any plane: none is shown any more. Answered with [`Event::Done`].
    Reset,
}

/// One plane's part of a frame: the buffer it shows and where and how strongly. A plane that only moves or fades names
/// the buffer it already shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Show {
    pub plane: u8,
    pub buffer: u8,
    /// Where the plane's top-left corner is on the panel, in panel pixels.
    pub at: (i32, i32),
    /// How opaque it is, 0 to 255, on top of what its pixels say.
    pub alpha: u8,
}

impl Show {
    /// A plane as large as the panel, at its corner, as its pixels say.
    pub const fn whole(plane: u8, buffer: u8) -> Self {
        Self {
            plane,
            buffer,
            at: (0, 0),
            alpha: 255,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// What the service offers.
    Offer(Offer),
    /// The request was carried out.
    Done,
    /// The frame `frame` is on the panel. `vsync_ns` is `CLOCK_MONOTONIC` when the service saw it there.
    Applied { frame: u64, vsync_ns: u64 },
    /// What differs between the layers on the panel and what the service put there; empty when nothing does.
    Checked { problems: Vec<String> },
    /// The request could not be carried out. The session is still usable, but a plane the request named may show the
    /// buffer it showed before or the one it was given, and which is not known until a `Present` that lists it, or a
    /// `Reset`, has been answered.
    Failed { message: String },
}

/// The service's end and the compositor's end of the socket: frames as in [`write_frame`] and [`read_frame`], and the
/// descriptors that came with them.
pub struct Channel {
    stream: UnixStream,
    /// Descriptors received and not yet taken, in the order they arrived: a frame that carries one is followed by a
    /// call to [`Channel::take_fd`].
    fds: VecDeque<OwnedFd>,
    frame: Vec<u8>,
}

impl Channel {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            fds: VecDeque::new(),
            frame: Vec::new(),
        }
    }

    pub fn connect(path: &Path) -> io::Result<Self> {
        UnixStream::connect(path).map(Self::new)
    }

    /// A reply that is not there after this long is an error, `None` for waiting for ever.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }

    /// Sends one message.
    pub fn send<T: Serialize>(&mut self, message: &T) -> io::Result<()> {
        write_frame(&mut &self.stream, message)
    }

    /// Sends one message with the descriptor `fd` attached: one `sendmsg`, so that the two arrive together.
    pub fn send_with_fd<T: Serialize>(
        &mut self,
        message: &T,
        fd: BorrowedFd<'_>,
    ) -> io::Result<()> {
        let mut frame = Vec::new();
        write_frame(&mut frame, message)?;
        send_fds(self.stream.as_raw_fd(), &frame, &[fd.as_raw_fd()])
    }

    /// Receives one message; `Ok(None)` when the other end has closed the connection.
    pub fn recv<T: for<'de> Deserialize<'de>>(&mut self) -> io::Result<Option<T>> {
        let mut buf = std::mem::take(&mut self.frame);
        let message = read_frame(self, &mut buf);
        self.frame = buf;
        message
    }

    /// The oldest descriptor that came in with a message and has not been taken.
    pub fn take_fd(&mut self) -> Option<OwnedFd> {
        self.fds.pop_front()
    }
}

impl Read for Channel {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let (n, received) = recv_fds(self.stream.as_raw_fd(), out)?;
        self.fds.extend(received);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{Seek, SeekFrom, Write};
    use std::os::fd::{AsFd, FromRawFd};

    fn pair() -> (Channel, Channel) {
        let (a, b) = UnixStream::pair().unwrap();
        (Channel::new(a), Channel::new(b))
    }

    /// An anonymous file holding `text`, for a descriptor that can be told apart by what it holds.
    fn file_with(text: &str) -> File {
        let mut file = tempfile();
        file.write_all(text.as_bytes()).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file
    }

    fn tempfile() -> File {
        let mut template = *b"/tmp/lumashow-display-XXXXXX\0";
        // SAFETY: `template` is a writable NUL-terminated `mkstemp` template.
        let fd = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
        assert!(fd >= 0, "mkstemp: {}", io::Error::last_os_error());
        // SAFETY: the file is unlinked at once and its descriptor is ours alone.
        unsafe {
            libc::unlink(template.as_ptr().cast());
            File::from_raw_fd(fd)
        }
    }

    fn read_all(fd: OwnedFd) -> String {
        let mut text = String::new();
        File::from(fd).read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn every_message_survives_the_wire() {
        let (mut a, mut b) = pair();
        let requests = [
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::Begin {
                planes: vec![
                    PlaneSpec {
                        format: PixelFormat::Rgba8888,
                        blend: Blend::Opaque,
                        width: 480,
                        height: 960,
                        matrix: None,
                    },
                    PlaneSpec {
                        format: PixelFormat::Yuyv,
                        blend: Blend::Opaque,
                        width: 320,
                        height: 180,
                        matrix: Some(ColorMatrix::Bt709Limited),
                    },
                ],
            },
            Request::Buffer {
                plane: 1,
                index: 2,
                stride: 480,
                len: 1_843_200,
            },
            Request::Present {
                frame: u64::MAX,
                show: vec![
                    Show::whole(0, 1),
                    Show {
                        plane: 1,
                        buffer: 0,
                        at: (-12, 700),
                        alpha: 128,
                    },
                ],
            },
            Request::Present {
                frame: 0,
                show: Vec::new(),
            },
            Request::Check,
            Request::Reset,
        ];
        for r in &requests {
            a.send(r).unwrap();
        }
        for r in &requests {
            assert_eq!(b.recv::<Request>().unwrap().as_ref(), Some(r));
        }
        let events = [
            Event::Offer(Offer {
                width: 480,
                height: 960,
                planes: 3,
                formats: vec![PixelFormat::Bgra8888],
                blends: vec![Blend::Opaque, Blend::Premultiplied],
                matrices: vec![ColorMatrix::Bt601Full, ColorMatrix::Bt709Limited],
                scaling: false,
                placement: true,
            }),
            Event::Done,
            Event::Applied {
                frame: 7,
                vsync_ns: 123_456_789,
            },
            Event::Checked {
                problems: vec!["layer 1 CON is 0x0".into()],
            },
            Event::Failed {
                message: "no".into(),
            },
        ];
        for e in &events {
            b.send(e).unwrap();
        }
        for e in &events {
            assert_eq!(a.recv::<Event>().unwrap().as_ref(), Some(e));
        }
        drop(b);
        assert_eq!(a.recv::<Event>().unwrap(), None, "a closed peer is an end");
    }

    #[test]
    fn a_descriptor_arrives_with_its_own_message() {
        let (mut a, mut b) = pair();
        let buffer = |index| Request::Buffer {
            plane: 0,
            index,
            stride: 480,
            len: 4096,
        };
        // Three buffers in a row, with a message that carries none between them, all sent before any is read: the
        // descriptors come out in the order of the messages that carried them, whatever the reads batch together.
        let (one, two, three) = (file_with("one"), file_with("two"), file_with("three"));
        a.send_with_fd(&buffer(0), one.as_fd()).unwrap();
        a.send(&Request::Check).unwrap();
        a.send_with_fd(&buffer(1), two.as_fd()).unwrap();
        a.send_with_fd(&buffer(2), three.as_fd()).unwrap();
        for (index, text) in [(0, "one"), (1, "two"), (2, "three")] {
            if index == 1 {
                assert_eq!(b.recv::<Request>().unwrap(), Some(Request::Check));
            }
            assert_eq!(b.recv::<Request>().unwrap(), Some(buffer(index)));
            let fd = b.take_fd().expect("the message's descriptor");
            assert_eq!(read_all(fd), text);
        }
        assert!(b.take_fd().is_none());
    }

    #[test]
    fn a_received_descriptor_is_a_new_one_of_the_same_file_and_does_not_leak_on_drop() {
        let (mut a, mut b) = pair();
        let file = file_with("shared");
        a.send_with_fd(&Request::Check, file.as_fd()).unwrap();
        assert_eq!(b.recv::<Request>().unwrap(), Some(Request::Check));
        let fd = b.take_fd().unwrap();
        assert_ne!(fd.as_raw_fd(), file.as_raw_fd());
        // Closing the received one leaves the sender's open.
        drop(fd);
        let mut again = String::new();
        (&file).read_to_string(&mut again).unwrap();
        assert_eq!(again, "shared");
    }

    #[test]
    fn a_reply_that_does_not_come_is_a_timeout_and_not_an_end() {
        let (mut a, _b) = pair();
        a.set_read_timeout(Some(Duration::from_millis(30))).unwrap();
        let err = a.recv::<Event>().unwrap_err();
        assert!(
            matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{err:?}"
        );
    }

    #[test]
    fn a_message_cut_short_is_an_error() {
        let (a, mut b) = pair();
        let mut frame = Vec::new();
        write_frame(&mut frame, &Request::Check).unwrap();
        frame.pop();
        (&a.stream).write_all(&frame).unwrap();
        drop(a);
        assert!(b.recv::<Request>().is_err());
    }
}
