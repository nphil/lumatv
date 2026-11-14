//! Sockets: a client connection (TLS or plain), descriptor passing, peer credentials and the connections to LG's gateway.

use rustls::ServerConnection;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::time::Duration;

/// Plaintext a TLS connection holds for the peer before it stops taking more.
const TLS_BUFFER: usize = 64 * 1024;

/// One client connection; reads and writes never block.
pub struct Conn {
    stream: TcpStream,
    tls: Option<Box<ServerConnection>>,
    pending: Vec<u8>,
    pub eof: bool,
}

impl Conn {
    pub fn new(fd: OwnedFd, tls: Option<&Arc<rustls::ServerConfig>>) -> io::Result<Conn> {
        // the descriptor may be a TCP socket (the set) or a Unix one (tests); both read and write alike
        let stream = TcpStream::from(fd);
        stream.set_nonblocking(true)?;
        let tls = match tls {
            Some(cfg) => {
                let mut c = ServerConnection::new(cfg.clone()).map_err(io::Error::other)?;
                c.set_buffer_limit(Some(TLS_BUFFER));
                Some(Box::new(c))
            }
            None => None,
        };
        Ok(Conn { stream, tls, pending: Vec::new(), eof: false })
    }

    pub fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    /// The client's address, for `_clientAddr` and the log.
    pub fn peer(&self) -> String {
        self.stream.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default()
    }

    /// Appends what has arrived to `into`; sets `eof` when the peer is done.
    pub fn read(&mut self, into: &mut Vec<u8>) -> io::Result<()> {
        let Some(tls) = self.tls.as_mut() else {
            return read_plain(&mut self.stream, into, &mut self.eof);
        };
        loop {
            match tls.read_tls(&mut self.stream) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(_) => {
                    let state = match tls.process_new_packets() {
                        Ok(s) => s,
                        Err(e) => {
                            // the alert rustls queued tells the peer why
                            let _ = tls.write_tls(&mut self.stream);
                            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
                        }
                    };
                    let n = state.plaintext_bytes_to_read();
                    if n > 0 {
                        let start = into.len();
                        into.resize(start + n, 0);
                        tls.reader().read_exact(&mut into[start..])?;
                    }
                    if state.peer_has_closed() {
                        self.eof = true;
                        return Ok(());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    pub fn queue(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
    }

    /// Bytes queued for the client and not yet handed to the kernel.
    pub fn backlog(&self) -> usize {
        self.pending.len()
    }

    pub fn wants_write(&self) -> bool {
        match &self.tls {
            Some(tls) => tls.wants_write() || (!tls.is_handshaking() && !self.pending.is_empty()),
            None => !self.pending.is_empty(),
        }
    }

    /// Writes as much as the socket takes.
    pub fn flush(&mut self) -> io::Result<()> {
        let Some(tls) = self.tls.as_mut() else {
            return write_plain(&mut self.stream, &mut self.pending);
        };
        loop {
            if !tls.is_handshaking() && !self.pending.is_empty() {
                let n = tls.writer().write(&self.pending)?;
                self.pending.drain(..n);
            }
            if !tls.wants_write() {
                return Ok(());
            }
            match tls.write_tls(&mut self.stream) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Ends the TLS layer cleanly once what is queued has gone.
    pub fn close_notify(&mut self) {
        if let Some(tls) = self.tls.as_mut() {
            let _ = tls.writer().write(&self.pending);
            self.pending.clear();
            tls.send_close_notify();
        }
    }
}

fn read_plain(stream: &mut TcpStream, into: &mut Vec<u8>, eof: &mut bool) -> io::Result<()> {
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                *eof = true;
                return Ok(());
            }
            Ok(n) => into.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

fn write_plain(stream: &mut TcpStream, pending: &mut Vec<u8>) -> io::Result<()> {
    while !pending.is_empty() {
        match stream.write(pending) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                pending.drain(..n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// A plain stream to LG's gateway or a local socket, non-blocking, with its own queues.
pub struct Pipe {
    pub stream: TcpStream,
    pub rx: Vec<u8>,
    out: Vec<u8>,
    pub eof: bool,
}

impl Pipe {
    /// Connects to LG's plain port; the loopback connect completes at once or fails.
    pub fn tcp(addr: &str) -> io::Result<Pipe> {
        let sa = addr.to_socket_addrs()?.next().ok_or(io::ErrorKind::AddrNotAvailable)?;
        let stream = TcpStream::connect_timeout(&sa, Duration::from_millis(500))?;
        Pipe::from_stream(stream)
    }

    pub fn unix(fd: OwnedFd) -> io::Result<Pipe> {
        Pipe::from_stream(TcpStream::from(fd))
    }

    fn from_stream(stream: TcpStream) -> io::Result<Pipe> {
        stream.set_nonblocking(true)?;
        Ok(Pipe { stream, rx: Vec::new(), out: Vec::new(), eof: false })
    }

    pub fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    pub fn read(&mut self) -> io::Result<()> {
        read_plain(&mut self.stream, &mut self.rx, &mut self.eof)
    }

    pub fn send(&mut self, data: &[u8]) {
        self.out.extend_from_slice(data);
    }

    pub fn wants_write(&self) -> bool {
        !self.out.is_empty()
    }

    pub fn backlog(&self) -> usize {
        self.out.len()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        write_plain(&mut self.stream, &mut self.out)
    }
}

/// Connects a Unix stream socket by path.
pub fn unix_connect(path: &str) -> io::Result<OwnedFd> {
    let s = std::os::unix::net::UnixStream::connect(path)?;
    Ok(OwnedFd::from(s))
}

/// The user id of the process at the other end of a Unix socket.
pub fn peer_uid(fd: RawFd) -> Option<u32> {
    // SAFETY: getsockopt fills the ucred it is given, no more than `len` bytes.
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &mut len);
        (rc == 0).then_some(cred.uid)
    }
}

/// Sends one byte with a descriptor attached.
#[cfg(test)]
pub fn send_fd(sock: RawFd, byte: u8, fd: RawFd) -> io::Result<()> {
    let mut data = [byte];
    let mut iov = libc::iovec { iov_base: data.as_mut_ptr().cast(), iov_len: 1 };
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut cbuf = vec![0u64; space.div_ceil(8)];
    // SAFETY: the msghdr points at the live iovec and control buffer, sized as above; the cmsg header is written inside it.
    unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        if libc::sendmsg(sock, &msg, libc::MSG_NOSIGNAL) == 1 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}

/// What one `recvmsg` brought: the bytes and the descriptor, if one came with them.
pub struct Received {
    pub data: Vec<u8>,
    pub fd: Option<OwnedFd>,
}

/// Receives up to `max` bytes and at most one descriptor (close-on-exec) without blocking; `Ok(None)` when nothing is waiting,
/// an empty `data` at end of stream.
pub fn recv_fd(sock: RawFd, max: usize) -> io::Result<Option<Received>> {
    let mut data = vec![0u8; max];
    let mut iov = libc::iovec { iov_base: data.as_mut_ptr().cast(), iov_len: max };
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32 * 4) } as usize;
    let mut cbuf = vec![0u64; space.div_ceil(8)];
    // SAFETY: the msghdr points at the live iovec and control buffer; every descriptor the kernel attached is taken into an
    // OwnedFd so none leaks.
    unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let n = libc::recvmsg(sock, &mut msg, libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC);
        if n < 0 {
            let e = io::Error::last_os_error();
            return match e.kind() {
                io::ErrorKind::WouldBlock => Ok(None),
                _ => Err(e),
            };
        }
        let mut fd = None;
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let count = ((*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / std::mem::size_of::<RawFd>();
                for i in 0..count {
                    let raw = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>().add(i));
                    let owned = OwnedFd::from_raw_fd(raw);
                    if fd.is_none() {
                        fd = Some(owned);
                    }
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
        data.truncate(n as usize);
        Ok(Some(Received { data, fd }))
    }
}

pub fn set_cloexec(fd: RawFd) {
    // SAFETY: fcntl on a descriptor the caller owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn descriptors_pass_with_their_byte_and_peers_are_known() {
        let (a, b) = UnixStream::pair().unwrap();
        let (x, mut y) = UnixStream::pair().unwrap();
        send_fd(a.as_raw_fd(), b'T', x.as_raw_fd()).unwrap();
        drop(x);
        let got = recv_fd(b.as_raw_fd(), 4).unwrap().unwrap();
        assert_eq!(got.data, b"T");
        let mut passed = UnixStream::from(got.fd.unwrap());
        passed.write_all(b"hi").unwrap();
        let mut buf = [0u8; 2];
        y.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi");
        assert!(recv_fd(b.as_raw_fd(), 4).unwrap().is_none());
        // SAFETY: geteuid has no preconditions.
        assert_eq!(peer_uid(a.as_raw_fd()), Some(unsafe { libc::geteuid() }));
    }
}
