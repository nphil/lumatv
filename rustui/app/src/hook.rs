//! LumaTV's task-manager hook: a Unix socket at `/tmp/<app id>/hook.sock` speaking JSON lines. `quit` ends the session the way LumaGame's
//! own clean exit does; `snapshot` writes the last presented frame as raw RGBA for the task manager's card. A thread owns the socket and
//! hands each request to the frame loop through a channel and a wake descriptor, so the loop never blocks on a client.
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{fs::{OpenOptionsExt, PermissionsExt}, net::{UnixDatagram, UnixListener, UnixStream}},
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, SyncSender}, Arc},
    thread::JoinHandle,
    time::Duration,
};

const SOCKET: &str = "hook.sock";
/// The task manager asks for the picture within 300 ms; the frame loop answers inside this.
const ANSWER_MS: u64 = 250;
/// A request is one short line; a client that sends more or stalls is dropped.
const LINE_MAX: u64 = 4096;
const CLIENT_MS: u64 = 1000;
/// The panel is 1920×1080: a card larger than that is a mistake, and the frame is only ever scaled down.
const MAX_W: u32 = 1920;
const MAX_H: u32 = 1080;

/// What the frame loop is asked to do.
pub enum Command {
    Quit,
    /// Writes the last presented UI frame, scaled to `w`×`h`, to `path`, and says how it went.
    Snapshot { path: PathBuf, w: u32, h: u32, reply: SyncSender<Result<(), String>> },
}

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Quit,
    Snapshot { path: PathBuf, w: u32, h: u32 },
}

fn parse(line: &str) -> Result<Request, String> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    let number = |key: &str| value.get(key).and_then(serde_json::Value::as_u64).ok_or_else(|| format!("{key} is missing or not a number"));
    match value.get("op").and_then(serde_json::Value::as_str) {
        Some("quit") => Ok(Request::Quit),
        Some("snapshot") => {
            let path = value.get("path").and_then(serde_json::Value::as_str).ok_or("path is missing")?;
            let (w, h) = (number("w")?, number("h")?);
            if w == 0 || h == 0 || w > u64::from(MAX_W) || h > u64::from(MAX_H) { return Err(format!("size must be within 1x1 and {MAX_W}x{MAX_H}")); }
            Ok(Request::Snapshot { path: PathBuf::from(path), w: w as u32, h: h as u32 })
        }
        Some(other) => Err(format!("unknown op {other}")),
        None => Err("op is missing".into()),
    }
}

/// The reply line to one request line. `send` gives a command to the frame loop and waits for its answer where it has one.
fn answer(line: &str, dir: &Path, send: &mut dyn FnMut(Command) -> Result<(), String>) -> String {
    let outcome = parse(line).and_then(|request| match request {
        Request::Quit => send(Command::Quit),
        // one file in the hook's own directory: nothing the task manager asks for can write elsewhere or grow the store
        Request::Snapshot { path, .. } if path.parent() != Some(dir) => Err(format!("path must be inside {}", dir.display())),
        Request::Snapshot { path, w, h } => {
            let (reply, answered) = mpsc::sync_channel(1);
            send(Command::Snapshot { path, w, h, reply })?;
            answered.recv_timeout(Duration::from_millis(ANSWER_MS)).map_err(|_| "the frame loop did not answer in time".to_string())?
        }
    });
    match outcome {
        Ok(()) => r#"{"ok":true}"#.into(),
        Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
    }
}

/// `rows` is `sw`×`sh` RGBA, top row first and premultiplied; the answer is `dw`×`dh` of the same, each pixel the mean of the block it
/// covers, with alpha made opaque (premultiplied over black is what the screen shows).
pub fn scale(rows: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = vec![255u8; dw * dh * 4];
    for y in 0..dh {
        let (y0, y1) = (y * sh / dh, ((y + 1) * sh / dh).max(y * sh / dh + 1).min(sh));
        for x in 0..dw {
            let (x0, x1) = (x * sw / dw, ((x + 1) * sw / dw).max(x * sw / dw + 1).min(sw));
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for px in rows[(sy * sw + x0) * 4..(sy * sw + x1) * 4].chunks_exact(4) {
                    for (s, c) in sum.iter_mut().zip(px) { *s += u32::from(*c); }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for (c, s) in out[(y * dw + x) * 4..][..3].iter_mut().zip(sum) { *c = (s / n) as u8; }
        }
    }
    out
}

/// Overwrites `path` with `rgba`: private to the app's user, one file that never grows past the card's size.
pub fn write_snapshot(path: &Path, rgba: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(rgba)
}

/// The listening socket and the thread behind it. Dropping it removes the socket and the snapshot.
pub struct Hook {
    dir: PathBuf,
    commands: Receiver<Command>,
    wake: UnixDatagram,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Hook {
    /// Listens at `/tmp/<app_id>/hook.sock`.
    pub fn start(app_id: &str) -> io::Result<Self> { Self::listen(Path::new("/tmp").join(app_id)) }

    fn listen(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let socket = dir.join(SOCKET);
        match fs::remove_file(&socket) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&socket)?;
        let (commands_tx, commands) = mpsc::channel();
        let (wake_tx, wake) = UnixDatagram::pair()?;
        wake_tx.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new().name("game-hook".into()).spawn({
            let (stop, dir) = (stop.clone(), dir.clone());
            move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::Acquire) { break; }
                    let Ok(stream) = stream else { continue };
                    let mut send = |command| {
                        commands_tx.send(command).map_err(|_| "the app is closing".to_string())?;
                        let _ = wake_tx.send(&[0]);
                        Ok(())
                    };
                    let _ = serve(stream, &dir, &mut send);
                }
            }
        })?;
        Ok(Self { dir, commands, wake, stop, thread: Some(thread) })
    }

    /// The descriptor the frame loop polls: readable when a command waits.
    pub fn fd(&self) -> std::os::fd::RawFd { std::os::fd::AsRawFd::as_raw_fd(&self.wake) }

    /// The commands that came in since the last call.
    pub fn poll(&self) -> Vec<Command> {
        let mut byte = [0u8; 16];
        while self.wake.recv(&mut byte).is_ok() {}
        self.commands.try_iter().collect()
    }
}

fn serve(stream: UnixStream, dir: &Path, send: &mut dyn FnMut(Command) -> Result<(), String>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(CLIENT_MS)))?;
    stream.set_write_timeout(Some(Duration::from_millis(CLIENT_MS)))?;
    let mut reader = BufReader::new((&stream).take(LINE_MAX));
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 { return Ok(()); }
        let mut reply = answer(line.trim(), dir, send);
        reply.push('\n');
        (&stream).write_all(reply.as_bytes())?;
        reader.get_mut().set_limit(LINE_MAX);
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // the thread waits in accept: one connection of our own lets it see the flag
        let _ = UnixStream::connect(self.dir.join(SOCKET));
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
        let _ = fs::remove_file(self.dir.join(SOCKET));
        let _ = fs::remove_file(self.dir.join("last.rgba"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_and_snapshot_lines_are_answered_and_the_one_file_stays_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("app");
        let hook = Hook::listen(dir.clone()).unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        // a 4x2 frame, top row first: left half red, right half blue
        let frame: Vec<u8> = (0..8).flat_map(|i| if i % 4 < 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }).collect();
        let service = std::thread::spawn(move || {
            let (mut seen, until) = ((0, 0), std::time::Instant::now() + Duration::from_secs(5));
            while seen.0 + seen.1 < 3 && std::time::Instant::now() < until {
                let _ = nix_wait(hook.fd());
                for command in hook.poll() {
                    match command {
                        Command::Quit => seen.0 += 1,
                        Command::Snapshot { path, w, h, reply } => {
                            seen.1 += 1;
                            let _ = reply.send(write_snapshot(&path, &scale(&frame, 4, 2, w, h)).map_err(|e| e.to_string()));
                        }
                    }
                }
            }
            (seen, hook)
        });
        let mut client = UnixStream::connect(dir.join(SOCKET)).unwrap();
        let snap = |w, h| format!("{{\"op\":\"snapshot\",\"path\":\"{}\",\"w\":{w},\"h\":{h}}}\n", dir.join("last.rgba").display());
        let mut lines = BufReader::new(client.try_clone().unwrap());
        let mut ask = |text: &str| { client.write_all(text.as_bytes()).unwrap(); let mut reply = String::new(); lines.read_line(&mut reply).unwrap(); reply };
        assert_eq!(ask("{\"op\":\"quit\",\"grace_ms\":3000}\n"), "{\"ok\":true}\n");
        assert_eq!(ask(&snap(2, 1)), "{\"ok\":true}\n");
        assert_eq!(fs::read(dir.join("last.rgba")).unwrap(), [255, 0, 0, 255, 0, 0, 255, 255]);
        assert_eq!(ask(&snap(1, 1)), "{\"ok\":true}\n");
        assert_eq!(fs::read(dir.join("last.rgba")).unwrap(), [127, 0, 127, 255], "the file is overwritten, not appended to");
        assert!(ask("{\"op\":\"snapshot\",\"path\":\"/etc/passwd\",\"w\":2,\"h\":1}\n").contains("\"ok\":false"));
        assert!(ask("{\"op\":\"snapshot\",\"path\":\"x\",\"w\":99999,\"h\":1}\n").contains("\"ok\":false"));
        assert!(ask("{\"op\":\"dance\"}\n").contains("unknown op"));
        assert!(ask("nonsense\n").contains("not JSON"));
        drop((client, lines));
        let (seen, hook) = service.join().unwrap();
        assert_eq!(seen, (1, 2));
        drop(hook);
        assert!(!dir.join(SOCKET).exists() && !dir.join("last.rgba").exists());
    }

    fn nix_wait(fd: i32) -> i32 {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        unsafe { libc::poll(&mut p, 1, 100) }
    }
}
