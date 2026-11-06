//! Passive readers only: LG's existing RAM stdlog and an independent critical-only /dev/kmsg cursor.
use crate::ctx::Ctx;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Default)]
struct State { available: bool, error: Option<String>, gaps: u64, dropped: u64 }
impl State {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"available":self.available,"error":self.error,"gaps":self.gaps,"dropped":self.dropped})
    }
    fn failed(&mut self, error: io::Error) { self.available = false; self.error = Some(error.to_string()); }
}
struct Stdlog { file: File, inode: u64, offset: u64, line: [u8; 4096], used: usize, truncated: bool }
impl Stdlog {
    fn open(path: &Path, initial: bool) -> io::Result<Self> {
        crate::logging::require_ram(path)?;
        let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        crate::logging::require_ram_fd(&file)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() { return Err(io::Error::other("native stdlog is not a regular RAM file")); }
        let inode = metadata.ino();
        let offset = if initial { file.seek(SeekFrom::End(0))? } else { 0 };
        Ok(Self { file, inode, offset, line: [0; 4096], used: 0, truncated: false })
    }
    /// Forwards what LG's log gained since the last tick; `Ok(true)` when it gained something.
    fn tick(&mut self, path: &Path, socket: &UnixDatagram, state: &mut State) -> io::Result<bool> {
        let mut read = false;
        let mut bytes = [0u8; 4096];
        for _ in 0..4 {
            let n = self.file.read(&mut bytes)?;
            if n == 0 {
                let metadata = std::fs::metadata(path)?;
                if metadata.ino() != self.inode {
                    // Compressed/rapid rotations are not replayed; this transition may contain a gap.
                    state.gaps = state.gaps.saturating_add(1);
                    *self = Self::open(path, false)?;
                } else if metadata.len() < self.offset {
                    state.gaps = state.gaps.saturating_add(1);
                    self.offset = self.file.seek(SeekFrom::Start(0))?;
                    self.used = 0;
                }
                break;
            }
            read = true;
            self.offset += n as u64;
            for &byte in &bytes[..n] {
                if byte == b'\n' {
                    send(socket, b"<14>lumatv-lg: ", &self.line[..self.used], state);
                    self.used = 0;
                    if self.truncated { state.gaps = state.gaps.saturating_add(1); self.truncated = false; }
                } else if self.used < self.line.len() {
                    self.line[self.used] = byte; self.used += 1;
                } else { self.truncated = true; }
            }
        }
        Ok(read)
    }
}
fn send(socket: &UnixDatagram, prefix: &[u8], payload: &[u8], state: &mut State) {
    let mut record = [0u8; crate::logging::RECORD_LIMIT];
    let n = payload.len().min(record.len() - prefix.len());
    record[..prefix.len()].copy_from_slice(prefix);
    record[prefix.len()..prefix.len()+n].copy_from_slice(&payload[..n]);
    if socket.send_to(&record[..prefix.len()+n], crate::logging::SOCKET).is_err() { state.dropped = state.dropped.saturating_add(1); }
}
fn kernel_open() -> io::Result<File> {
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open("/dev/kmsg")?;
    // /dev/kmsg SEEK_END is this descriptor's cursor only; never consumes the global syslog cursor.
    file.seek(SeekFrom::End(0))?;
    Ok(file)
}
fn kernel_record(record: &[u8]) -> Option<(u8, u64, &[u8])> {
    let semicolon = record.iter().position(|b| *b == b';')?;
    let header = std::str::from_utf8(&record[..semicolon]).ok()?;
    let mut fields = header.split(',');
    let priority: u8 = fields.next()?.parse().ok()?;
    let sequence = fields.next()?.parse().ok()?;
    Some((priority, sequence, &record[semicolon+1..]))
}
fn kernel_tick(file: &mut File, last: &mut Option<u64>, socket: &UnixDatagram, state: &mut State) -> io::Result<bool> {
    let mut read = false;
    let mut record = [0u8; 8192];
    for _ in 0..4 {
        match file.read(&mut record) {
            Ok(0) => break,
            Ok(n) => {
                read = true;
                let Some((priority, sequence, payload)) = kernel_record(&record[..n]) else { state.gaps = state.gaps.saturating_add(1); continue; };
                if last.is_some_and(|previous| sequence != previous + 1) { state.gaps = state.gaps.saturating_add(1); }
                *last = Some(sequence);
                if priority & 7 <= 4 {
                    let prefix: &[u8] = match priority & 7 { 0 => b"<8>lumatv-kernel: ", 1 => b"<9>lumatv-kernel: ", 2 => b"<10>lumatv-kernel: ", 3 => b"<11>lumatv-kernel: ", _ => b"<12>lumatv-kernel: " };
                    send(socket, prefix, payload, state);
                }
            }
            Err(error) if error.raw_os_error() == Some(libc::EPIPE) => { state.gaps = state.gaps.saturating_add(1); *last = None; },
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => break,
            Err(error) => return Err(error),
        }
    }
    Ok(read)
}

pub fn run(ctx: Ctx) {
    let socket = match UnixDatagram::unbound().and_then(|socket| { socket.set_nonblocking(true)?; crate::logging::socket_buffer(&socket)?; Ok(socket) }) {
        Ok(socket) => socket,
        Err(error) => { ctx.hub.set("loggingSources", serde_json::json!({"error":error.to_string()})); return; }
    };
    let path = Path::new("/var/log/messages");
    let mut lg = None;
    let mut kernel = None;
    let mut lg_state = State::default();
    let mut kernel_state = State::default();
    let mut last_sequence = None;
    let mut retry = Instant::now();
    let mut publish = Instant::now();
    while !ctx.stop.is_set() {
        if Instant::now() >= retry {
            if lg.is_none() {
                match Stdlog::open(path, true) { Ok(reader) => { lg = Some(reader); lg_state.available = true; lg_state.error = None; }, Err(error) => lg_state.failed(error) }
            }
            if kernel.is_none() {
                match kernel_open() { Ok(reader) => { kernel = Some(reader); kernel_state.available = true; kernel_state.error = None; }, Err(error) => kernel_state.failed(error) }
            }
            retry = Instant::now() + Duration::from_secs(5);
        }
        let mut read = false;
        if let Some(reader) = lg.as_mut() {
            match reader.tick(path, &socket, &mut lg_state) { Ok(got) => read |= got, Err(error) => { lg_state.failed(error); lg = None; publish = Instant::now(); } }
        }
        if let Some(reader) = kernel.as_mut() {
            match kernel_tick(reader, &mut last_sequence, &socket, &mut kernel_state) { Ok(got) => read |= got, Err(error) => { kernel_state.failed(error); kernel = None; publish = Instant::now(); } }
        }
        if Instant::now() >= publish {
            ctx.hub.set("loggingSources", serde_json::json!({"lgStdlog":lg_state.json(),"criticalKernel":kernel_state.json(),"kernelSeverityMax":4,"startsAtEnd":true,"lgCoverage":"accepted stdlog only; compressed rotations are not replayed"}));
            publish = Instant::now() + Duration::from_secs(5);
        }
        // 20 ms while lines arrive, 250 ms when none did: a quiet log no longer wakes the helper 50 times a second
        if ctx.stop.sleep(Duration::from_millis(if read { 20 } else { 250 })) { break; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_header_preserves_sequence_and_severity() {
        let (priority, sequence, payload) = kernel_record(b"4,123,456,-;critical-marker\n").unwrap();
        assert_eq!((priority & 7, sequence, payload), (4, 123, b"critical-marker\n".as_slice()));
        assert!(kernel_record(b"invalid").is_none());
    }
    #[test]
    fn native_reader_starts_at_end_and_handles_rotation_without_replay() {
        use std::io::Write;
        let dir = tempfile::tempdir_in("/dev/shm").unwrap();
        let path = dir.path().join("messages");
        std::fs::write(&path, b"old boot data\n").unwrap();
        let mut reader = Stdlog::open(&path, true).unwrap();
        assert_eq!(reader.offset, 14);
        OpenOptions::new().append(true).open(&path).unwrap().write_all(b"new accepted data\n").unwrap();
        let socket = UnixDatagram::unbound().unwrap(); socket.set_nonblocking(true).unwrap();
        let mut state = State::default();
        reader.tick(&path, &socket, &mut state).unwrap();
        assert_eq!(state.dropped, 1);
        std::fs::rename(&path, dir.path().join("messages.0")).unwrap();
        std::fs::write(&path, b"new generation\n").unwrap();
        reader.tick(&path, &socket, &mut state).unwrap();
        reader.tick(&path, &socket, &mut state).unwrap();
        assert_eq!((state.gaps, state.dropped), (1, 2));
    }
}
