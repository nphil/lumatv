//! Child diagnostics are drained independently and sent to the sole RAM collector. A child never inherits a retained log file.

use std::fs::File;
use std::fmt;
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};

pub fn pipe(tag: &'static str, prefix: &'static str) -> Option<OwnedFd> {
    let mut fds = [0; 2];
    // Only the child's write end is nonblocking: a stalled drain must never hold a UI or shutdown thread.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 { return None; }
    let (mut reader, writer) = unsafe { (File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    if unsafe { libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK) } < 0 { return None; }
    std::thread::Builder::new().name("client-log".into()).spawn(move || {
        let mut buf = [0; 8192];
        let mut line = [0; 4096];
        let mut len = 0;
        let mut cut = false;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => { if len > 0 || cut { emit(tag, prefix, &line[..len], cut); } break; }
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            for &byte in &buf[..n] {
                if byte == b'\n' {
                    emit(tag, prefix, &line[..len], cut);
                    len = 0;
                    cut = false;
                } else if len < line.len() { line[len] = byte; len += 1; }
                else { cut = true; }
            }
        }
    }).ok()?;
    Some(writer)
}

struct NativeLine<'a>(&'a [u8]);

impl fmt::Display for NativeLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rest = self.0;
        loop {
            let error = match std::str::from_utf8(rest) {
                Ok(text) => return f.write_str(text),
                Err(error) => error,
            };
            let (valid, invalid) = rest.split_at(error.valid_up_to());
            // valid_up_to identifies a complete, already-validated UTF-8 prefix.
            f.write_str(unsafe { std::str::from_utf8_unchecked(valid) })?;
            f.write_str("�")?;
            let Some(len) = error.error_len() else { return Ok(()); };
            rest = &invalid[len..];
        }
    }
}

fn emit(tag: &str, prefix: &str, bytes: &[u8], cut: bool) {
    let severity = if bytes.windows(7).any(|part| part == b" ERROR ") || bytes.starts_with(b"error:") { 3 }
        else if bytes.windows(6).any(|part| part == b" WARN ") || bytes.starts_with(b"warn:") { 4 } else { 6 };
    crate::logsink::send(tag, severity, format_args!("{prefix}{}{}", NativeLine(bytes), if cut { "…" } else { "" }));
}
