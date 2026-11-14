//! xinetd's server for port 3001. It hands the connection on its standard input to the gateway through
//! `$SSGW_RUN_DIR/accept.sock`, or runs LG's `nsl` on it whenever the gateway is not there to take it.
//!
//! xinetd puts the connection on descriptors 0, 1 and 2, so nothing here may print.

use std::ffi::OsString;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::Duration;

const RUN_DIR: &str = "/tmp/lumatv-ssgw";
const NSL: &str = "/usr/bin/nsl";
/// The gateway answers at once when it is up; past this, LG's gateway serves the connection.
const READY_WAIT: Duration = Duration::from_millis(200);
const TAKEN_WAIT: Duration = Duration::from_secs(3);

enum Outcome {
    Taken,
    /// The gateway said it was ready but never confirmed: it may hold the connection, so LG must not answer it too.
    Lost,
    Fallback,
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().map(|a| a.as_bytes()) == Some(b"3001") {
        let dir = std::env::var_os("SSGW_RUN_DIR").unwrap_or_else(|| RUN_DIR.into());
        match hand_over(Path::new(&dir)) {
            Outcome::Taken => return,
            Outcome::Lost => std::process::exit(1),
            Outcome::Fallback => {}
        }
    }
    let nsl = std::env::var_os("SSGW_NSL").unwrap_or_else(|| NSL.into());
    let _ = std::process::Command::new(nsl).arg0("nsl").args(&args).exec();
    std::process::exit(127);
}

fn hand_over(dir: &Path) -> Outcome {
    // only a directory others cannot write into is trusted to hold the gateway's socket
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return Outcome::Fallback };
    if !meta.is_dir() || meta.mode() & 0o022 != 0 {
        return Outcome::Fallback;
    }
    let Ok(sock) = UnixStream::connect(dir.join("accept.sock")) else { return Outcome::Fallback };
    match peer_uid(sock.as_raw_fd()) {
        Some(uid) if uid == 0 || uid == meta.uid() => {}
        _ => return Outcome::Fallback,
    }
    let mut byte = [0u8; 1];
    let _ = sock.set_read_timeout(Some(READY_WAIT));
    if !matches!((&sock).read(&mut byte), Ok(1)) || byte[0] != b'R' {
        return Outcome::Fallback;
    }
    if send_fd(sock.as_raw_fd(), b'T', 0).is_err() {
        return Outcome::Fallback;
    }
    let _ = sock.set_read_timeout(Some(TAKEN_WAIT));
    match (&sock).read(&mut byte) {
        Ok(1) if byte[0] == b'T' => Outcome::Taken,
        _ => Outcome::Lost,
    }
}

fn peer_uid(fd: RawFd) -> Option<u32> {
    // SAFETY: getsockopt writes at most `len` bytes into the zeroed ucred.
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let r = libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &mut len);
        (r == 0).then_some(cred.uid)
    }
}

/// One byte with `fd` attached.
fn send_fd(sock: RawFd, byte: u8, fd: RawFd) -> io::Result<()> {
    let mut data = [byte];
    let mut iov = libc::iovec { iov_base: data.as_mut_ptr().cast(), iov_len: 1 };
    // SAFETY: CMSG_SPACE is a size computation.
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut control = vec![0u64; space.div_ceil(8)];
    // SAFETY: the message points at live locals; the control buffer holds one SCM_RIGHTS header and descriptor.
    unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        if libc::sendmsg(sock, &msg, libc::MSG_NOSIGNAL) == 1 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}
