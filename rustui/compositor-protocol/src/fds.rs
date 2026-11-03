//! Descriptors over a Unix stream socket (`SCM_RIGHTS`): one `sendmsg` that carries them with the first byte of a message, and
//! a reader that takes messages and the descriptors that came with them off a socket a few bytes at a time.
//!
//! The two kinds of socket that carry dma-bufs here (the display service's and the frame endpoint's) share this; the framing
//! on top is the app protocol's ([`write_frame`], [`read_frame`]).

use crate::{MAX_FRAME, read_frame};
use serde::Deserialize;
use std::collections::VecDeque;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// The descriptors one `recvmsg` may bring: a message that carries them has at most [`MAX_FDS`], and the reader may find the
/// descriptors of several messages that arrived together.
pub(crate) const MAX_FDS_PER_READ: usize = 16;

/// The most descriptors one message may carry.
pub const MAX_FDS: usize = 8;

/// `CMSG_SPACE` for `count` descriptors.
fn fd_space(count: usize) -> usize {
    // SAFETY: a pure computation on the argument.
    unsafe { libc::CMSG_SPACE((count * size_of::<RawFd>()) as u32) as usize }
}

/// Writes `bytes` to the stream socket `socket` in one `sendmsg` with `fds` as ancillary data (none: a plain write).
pub(crate) fn send_fds(socket: RawFd, bytes: &[u8], fds: &[RawFd]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME as usize + 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    if fds.len() > MAX_FDS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a message carries at most {MAX_FDS} descriptors"),
        ));
    }
    let mut control = vec![
        0u8;
        if fds.is_empty() {
            0
        } else {
            fd_space(fds.len())
        }
    ];
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: a zeroed `msghdr` is valid (null pointers, zero lengths); the fields are then pointed at live buffers.
    let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
    header.msg_iov = &mut iov;
    header.msg_iovlen = 1;
    if !fds.is_empty() {
        header.msg_control = control.as_mut_ptr().cast();
        header.msg_controllen = control.len() as _;
        // SAFETY: `control` is `CMSG_SPACE` of the descriptors, so the first header and their bytes are inside it.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&header);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(size_of_val(fds) as u32) as _;
            std::ptr::copy_nonoverlapping(
                fds.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                size_of_val(fds),
            );
        }
    }
    let mut sent = 0;
    // The descriptors travel with the first byte; a short write (a full socket buffer) sends the rest as plain data.
    while sent < bytes.len() {
        let n = if sent == 0 {
            // SAFETY: `header` points at live buffers for the duration of the call.
            unsafe { libc::sendmsg(socket, &header, libc::MSG_NOSIGNAL) }
        } else {
            // SAFETY: the remainder of a live buffer.
            unsafe {
                libc::send(
                    socket,
                    bytes[sent..].as_ptr().cast(),
                    bytes.len() - sent,
                    libc::MSG_NOSIGNAL,
                )
            }
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        sent += n as usize;
    }
    Ok(())
}

/// One `recvmsg` into `out`: how many bytes came, and the descriptors that came with them (close-on-exec).
pub(crate) fn recv_fds(socket: RawFd, out: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
    let mut control = vec![0u8; fd_space(MAX_FDS_PER_READ)];
    let mut iov = libc::iovec {
        iov_base: out.as_mut_ptr().cast(),
        iov_len: out.len(),
    };
    // SAFETY: as in `send_fds`.
    let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
    header.msg_iov = &mut iov;
    header.msg_iovlen = 1;
    header.msg_control = control.as_mut_ptr().cast();
    header.msg_controllen = control.len() as _;
    let n = loop {
        // SAFETY: `header` points at live buffers for the duration of the call.
        let n = unsafe { libc::recvmsg(socket, &mut header, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break n as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut fds = Vec::new();
    // SAFETY: the kernel filled `control` with well-formed headers, `msg_controllen` bytes of them.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&header);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
                for k in 0..bytes / size_of::<RawFd>() {
                    // Each is a descriptor the kernel made for this call; nothing else owns it.
                    fds.push(OwnedFd::from_raw_fd(std::ptr::read_unaligned(data.add(k))));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&header, cmsg);
        }
    }
    if header.msg_flags & libc::MSG_CTRUNC != 0 {
        // `fds` closes the ones that did arrive as it goes out of scope.
        return Err(io::Error::other(
            "descriptors were lost: the ancillary data did not fit",
        ));
    }
    Ok((n, fds))
}

/// What one read of a socket found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// Bytes (and perhaps descriptors) arrived.
    Data,
    /// Nothing was there to read (a non-blocking socket).
    Empty,
    /// The other end has closed the connection.
    Closed,
}

/// Takes messages (and the descriptors that came with them) off a non-blocking stream socket as they arrive, a few bytes at a
/// time if that is how they come. The descriptors are kept in the order they arrived, and a message that carries some is
/// followed by a call to [`Reader::take_fds`]; the reader owns every descriptor from the moment it arrives, so a peer that
/// attaches descriptors to a message that wants none costs the reader nothing but their closing.
#[derive(Default)]
pub struct Reader {
    buf: Vec<u8>,
    fds: VecDeque<OwnedFd>,
}

/// How much one read takes off the socket.
const READ_CHUNK: usize = 4096;

/// How many descriptors may wait for messages at once: a peer that sends more than a few messages' worth without ever asking
/// for them is cut off rather than allowed to fill the descriptor table.
const MAX_WAITING_FDS: usize = 2 * MAX_FDS;

impl Reader {
    /// Reads once from `socket`. An error is the connection's end (including a peer that sent more descriptors than it may).
    pub fn fill(&mut self, socket: RawFd) -> io::Result<Fill> {
        let mut chunk = [0u8; READ_CHUNK];
        match recv_fds(socket, &mut chunk) {
            Ok((0, received)) if received.is_empty() => Ok(Fill::Closed),
            Ok((n, received)) => {
                self.buf.extend_from_slice(&chunk[..n]);
                self.fds.extend(received);
                if self.fds.len() > MAX_WAITING_FDS {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many descriptors waiting for messages",
                    ));
                }
                Ok(Fill::Data)
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(Fill::Empty),
            Err(e) => Err(e),
        }
    }

    /// The next whole message, if one has arrived. An error is a frame that is too long or does not decode: the stream is out
    /// of step and the connection is over.
    pub fn message<T: for<'de> Deserialize<'de>>(&mut self) -> io::Result<Option<T>> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes(self.buf[..4].try_into().expect("four bytes"));
        if len > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a frame of {len} bytes exceeds the limit of {MAX_FRAME}"),
            ));
        }
        let end = 4 + len as usize;
        if self.buf.len() < end {
            return Ok(None);
        }
        let mut frame = &self.buf[..end];
        let mut scratch = Vec::new();
        let message = read_frame(&mut frame, &mut scratch);
        self.buf.drain(..end);
        message
    }

    /// The `count` oldest descriptors that came in with messages, or `None` (and none is taken) when fewer are waiting.
    pub fn take_fds(&mut self, count: usize) -> Option<Vec<OwnedFd>> {
        if self.fds.len() < count {
            return None;
        }
        Some(self.fds.drain(..count).collect())
    }

    /// How many descriptors are waiting for a message to claim them.
    pub fn waiting_fds(&self) -> usize {
        self.fds.len()
    }

    /// Throws away every descriptor still waiting (a message that wanted none came with some).
    pub fn drop_fds(&mut self) {
        self.fds.clear();
    }
}
