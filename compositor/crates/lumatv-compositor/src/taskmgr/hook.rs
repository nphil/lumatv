//! An app's hook for the task manager: a Unix socket at `<root>/<app id>/hook.sock` (root is `/tmp`) speaking JSON lines, served by LumaGame
//! (`org.nphil.lumagame`). No socket means no hook, and the task manager does what it does for any app.
//!
//! `{"op":"quit","grace_ms":3000}` ends the session cleanly; `{"op":"snapshot","path":…,"w":640,"h":360}` writes the app's last UI frame as raw
//! RGBA (`w*h*4` bytes) to `path`; each is answered `{"ok":true}`. Both calls here block for at most [`ANSWER`], so they run on a thread of
//! their own (`switcher/hook.rs`), never on the frame loop.

use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const SOCKET: &str = "hook.sock";
const PICTURE: &str = "last.rgba";
/// What `quit` is told it has, and how long the task manager waits for the app to be gone before it closes it the ordinary way.
pub const QUIT_GRACE: Duration = Duration::from_secs(3);
/// How long a hook has to answer, and to have written its picture.
pub const ANSWER: Duration = Duration::from_millis(300);
/// The picture asked for.
pub const CARD: (u32, u32) = (640, 360);
/// A picture whose mean red, green and blue is under this (of 255) is black: a game stream or a video on the hardware layer is not in it.
const BLACK: u64 = 5;
/// A reply is one short line.
const LINE_MAX: usize = 4096;

/// What a `snapshot` came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Snapshot {
    /// `CARD` RGBA, top row first.
    Pixels(Vec<u8>),
    /// The hook answered with a black frame: the app has no picture to show.
    Black,
    /// No answer, no file, or a file of the wrong size: nothing is known.
    Failed,
}

/// An app id is a directory name: one that could leave the root is not asked.
fn valid(app: &str) -> bool {
    !app.is_empty() && app != "." && app != ".." && !app.contains('/')
}

fn socket(root: &Path, app: &str) -> PathBuf {
    root.join(app).join(SOCKET)
}

/// Whether the app serves a hook (one `stat`).
pub fn present(root: &Path, app: &str) -> bool {
    valid(app) && fs::symlink_metadata(socket(root, app)).is_ok_and(|m| m.file_type().is_socket())
}

/// Asks the app to end its session. Whether it said so.
pub fn quit(root: &Path, app: &str) -> bool {
    let request = json!({"op": "quit", "grace_ms": QUIT_GRACE.as_millis() as u64});
    valid(app) && exchange(&socket(root, app), &request, Instant::now() + ANSWER).unwrap_or(false)
}

/// Asks the app for its last frame. The file it writes is read and deleted: nothing stays on the set.
pub fn snapshot(root: &Path, app: &str) -> Snapshot {
    if !valid(app) {
        return Snapshot::Failed;
    }
    let (w, h) = CARD;
    let path = root.join(app).join(PICTURE);
    let request = json!({"op": "snapshot", "path": path.to_string_lossy(), "w": w, "h": h});
    let said = exchange(&socket(root, app), &request, Instant::now() + ANSWER).unwrap_or(false);
    let pixels = if said { fs::read(&path).ok() } else { None };
    let _ = fs::remove_file(&path);
    match pixels {
        Some(p) if p.len() == w as usize * h as usize * 4 => if black(&p) { Snapshot::Black } else { Snapshot::Pixels(p) },
        _ => Snapshot::Failed,
    }
}

/// Whether the mean of the red, green and blue of `rgba` is under [`BLACK`].
pub fn black(rgba: &[u8]) -> bool {
    let sum: u64 = rgba.chunks_exact(4).map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2])).sum();
    sum < BLACK * 3 * (rgba.len() as u64 / 4)
}

/// One request line and its reply line; whether the reply says `"ok":true`. Everything ends by `deadline`.
fn exchange(path: &Path, request: &Value, deadline: Instant) -> io::Result<bool> {
    let left = || deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut));
    let mut stream = connect(path)?;
    stream.set_write_timeout(Some(left()?))?;
    stream.write_all(format!("{request}\n").as_bytes())?;
    let (mut line, mut chunk) = (Vec::new(), [0u8; 256]);
    while !line.contains(&b'\n') && line.len() < LINE_MAX {
        stream.set_read_timeout(Some(left()?))?;
        match stream.read(&mut chunk)? {
            0 => break,
            n => line.extend_from_slice(&chunk[..n]),
        }
    }
    let reply: Value = serde_json::from_slice(line.split(|b| *b == b'\n').next().unwrap_or_default()).map_err(io::Error::other)?;
    Ok(reply.get("ok").and_then(Value::as_bool) == Some(true))
}

/// Connects without waiting: `UnixStream::connect` blocks without bound while the listener's queue is full, as an app that stopped has it.
fn connect(path: &Path) -> io::Result<UnixStream> {
    // SAFETY: an all-zero `sockaddr_un` is a valid value.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (to, from) in addr.sun_path.iter_mut().zip(bytes) {
        *to = *from as libc::c_char;
    }
    // SAFETY: a new socket; the descriptor is checked and owned by the stream from here on.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a socket this function just made and nothing else owns.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    // SAFETY: `addr` is a filled `sockaddr_un` and the length is its size.
    let connected = unsafe { libc::connect(fd, (&raw const addr).cast(), std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t) };
    if connected != 0 {
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_black_when_its_mean_colour_is_under_five() {
        let frame = |c: [u8; 3]| c.iter().chain(&[255]).copied().cycle().take(16).collect::<Vec<u8>>();
        assert!(black(&frame([0, 0, 0])));
        assert!(black(&frame([4, 4, 4])));
        assert!(!black(&frame([5, 5, 5])));
        assert!(!black(&frame([0, 0, 15])), "the mean of the three channels is 5");
        assert!(black(&frame([0, 0, 14])));
        let mut mostly = vec![0u8; 400];
        mostly.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        mostly[0] = 255;
        assert!(black(&mostly), "one lit pixel in a hundred is black");
    }

    #[test]
    fn an_id_that_could_leave_the_root_is_not_asked() {
        let root = Path::new("/nonexistent-hook-root");
        for id in ["", ".", "..", "../x", "a/b"] {
            assert!(!valid(id), "{id:?}");
            assert!(!present(root, id));
        }
        assert!(valid("org.nphil.lumagame"));
        assert!(!present(root, "org.nphil.lumagame"), "no socket, no hook");
        assert_eq!(snapshot(root, "org.nphil.lumagame"), Snapshot::Failed);
        assert!(!quit(root, "org.nphil.lumagame"));
    }
}
