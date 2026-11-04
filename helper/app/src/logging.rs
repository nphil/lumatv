//! Single bounded RAM log owner. UDP is best-effort, never a connection guarantee.
use crate::ctx::Ctx;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::Duration;

pub const SOCKET: &str = "/tmp/lumatv-log.sock";
pub const DIRECTORY: &str = "/tmp/lumatv-logs";
pub const RAM_LIMIT: u64 = 1_048_576;
pub const RECORD_LIMIT: usize = 4096;
const FILE_LIMIT: u64 = RAM_LIMIT / 2;
const FILE_HEADER: &[u8] = b"LUMATV-LOG-V1\n";

pub fn valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok() || (!host.is_empty() && host.len() <= 253 && host.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }))
}

use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};

fn trusted_owner() -> u32 {
    #[allow(unsafe_code)]
    unsafe { libc::geteuid() }
}
fn private_file(file: &File, maximum: u64) -> io::Result<std::fs::Metadata> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != trusted_owner() || metadata.mode() & 0o7777 != 0o600 || metadata.nlink() != 1 || metadata.len() > maximum {
        return Err(io::Error::other("retained log entry is not a private owned single-link regular file"));
    }
    require_ram_fd(file)?;
    Ok(metadata)
}

struct LogDirectory(File);
impl LogDirectory {
    fn open(path: &Path, create: bool) -> io::Result<Self> {
        if create {
            match std::fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {},
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(error),
            }
        }
        let file = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.uid() != trusted_owner() || metadata.mode() & 0o7777 != 0o700 {
            return Err(io::Error::other("logging directory is not a private owned directory"));
        }
        require_ram_fd(&file)?;
        Ok(Self(file))
    }
    #[allow(unsafe_code)]
    fn open_file(&self, name: &std::ffi::CStr, flags: libc::c_int, maximum: u64) -> io::Result<File> {
        // The verified directory descriptor pins every retained entry even if its pathname changes.
        let descriptor = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC, 0o600) };
        if descriptor < 0 { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_fd(descriptor) };
        private_file(&file, maximum)?;
        Ok(file)
    }
    fn optional_log(&self, name: &std::ffi::CStr) -> io::Result<Option<File>> {
        match self.open_file(name, libc::O_RDONLY, FILE_LIMIT) {
            Ok(mut file) => { verify_log_header(&mut file)?; Ok(Some(file)) },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn create_log(&self, name: &std::ffi::CStr) -> io::Result<File> {
        let mut file = self.open_file(name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_APPEND, FILE_LIMIT)?;
        file.write_all(FILE_HEADER)?;
        Ok(file)
    }
    fn current_log(&self) -> io::Result<File> {
        match self.create_log(c"current.log") {
            Ok(file) => Ok(file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let mut file = self.open_file(c"current.log", libc::O_RDWR | libc::O_APPEND, FILE_LIMIT)?;
                verify_log_header(&mut file)?;
                Ok(file)
            },
            Err(error) => Err(error),
        }
    }
    #[allow(unsafe_code)]
    fn lock(&self) -> io::Result<File> {
        let file = self.open_file(c"owner.lock", libc::O_RDWR | libc::O_CREAT, 128)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 { return Err(io::Error::last_os_error()); }
        Ok(file)
    }
    #[allow(unsafe_code)]
    fn rotate(&self, current: &File) -> io::Result<File> {
        private_file(current, FILE_LIMIT)?;
        let on_path = self.open_file(c"current.log", libc::O_RDONLY, FILE_LIMIT)?;
        let expected = current.metadata()?;
        let actual = on_path.metadata()?;
        if (expected.dev(), expected.ino()) != (actual.dev(), actual.ino()) { return Err(io::Error::other("current log identity changed")); }
        let _previous = self.optional_log(c"previous.log")?;
        if unsafe { libc::renameat(self.0.as_raw_fd(), c"current.log".as_ptr(), self.0.as_raw_fd(), c"previous.log".as_ptr()) } != 0 { return Err(io::Error::last_os_error()); }
        self.create_log(c"current.log")
    }
}

fn verify_log_header(file: &mut File) -> io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut header = [0u8; FILE_HEADER.len()];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header).map_err(|_| io::Error::other("retained file has no collector ownership header"))?;
    if header != FILE_HEADER { return Err(io::Error::other("retained file has no collector ownership header")); }
    Ok(())
}
struct Retained { file: File, bytes: u64, dir: LogDirectory }
impl Retained {
    #[cfg(test)]
    fn open(dir: &Path) -> io::Result<Self> { Self::in_directory(LogDirectory::open(dir, true)?) }
    fn in_directory(dir: LogDirectory) -> io::Result<Self> {
        let _previous = dir.optional_log(c"previous.log")?;
        let file = dir.current_log()?;
        let bytes = file.metadata()?.len();
        Ok(Self { file, bytes, dir })
    }
    fn append(&mut self, message: &[u8]) -> io::Result<()> {
        private_file(&self.file, FILE_LIMIT)?;
        let message = &message[..message.len().min(RECORD_LIMIT)];
        if self.bytes + message.len() as u64 + 1 > FILE_LIMIT {
            self.file = self.dir.rotate(&self.file)?;
            self.bytes = FILE_HEADER.len() as u64;
        }
        self.file.write_all(message)?;
        self.file.write_all(b"\n")?;
        self.bytes += message.len() as u64 + 1;
        Ok(())
    }
}

pub fn run(ctx: Ctx) {
    let result = run_at(&ctx, Path::new(DIRECTORY), Path::new(SOCKET));
    if let Err(error) = result {
        ctx.hub.set("logging", status(&ctx, false, Some(error.to_string())));
    }
}
fn status(ctx: &Ctx, active: bool, error: Option<String>) -> serde_json::Value {
    serde_json::json!({"enabled":ctx.settings.flag("base.logging", "enabled").unwrap_or(false),
        "active":active,"server":{"host":ctx.settings.text("base.logging", "host").unwrap_or_default(),
        "port":ctx.settings.integer("base.logging", "port").unwrap_or(514)},"transport":"udp","error":error,"ramLimitBytes":RAM_LIMIT})
}
pub(crate) fn run_at(ctx: &Ctx, dir: &Path, socket_path: &Path) -> io::Result<()> {
    let directory = LogDirectory::open(dir, true)?;
    let mut lock = directory.lock()?;
    let previous_socket = read_socket_marker(&mut lock)?;
    prepare_socket(socket_path, previous_socket)?;
    let input = UnixDatagram::bind(socket_path)?;
    let socket_cleanup = SocketCleanup::bound(socket_path)?;
    write_socket_marker(&mut lock, &socket_cleanup)?;
    let _socket_cleanup = socket_cleanup;
    socket_buffer(&input)?;
    input.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut retained = Retained::in_directory(directory)?;
    let mut applied = None;
    let mut output = None;
    let mut record = [0u8; RECORD_LIMIT];
    let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let notify = changed.clone();
    ctx.settings.on_change("base.logging", move || notify.store(true, std::sync::atomic::Ordering::Release));
    while !ctx.stop.is_set() {
        if changed.swap(false, std::sync::atomic::Ordering::AcqRel) {
        let enabled = ctx.settings.flag("base.logging", "enabled").unwrap_or(false);
        let host = ctx.settings.text("base.logging", "host").unwrap_or_default();
        let port = ctx.settings.integer("base.logging", "port").unwrap_or(514) as u16;
        let desired = (enabled, host, port);
        if applied.as_ref() != Some(&desired) {
            output = None;
            let mut error = None;
            if enabled {
                match resolve_bounded(&desired.1, port, &ctx.stop)
                    .and_then(|target| {
                        let socket = UdpSocket::bind(if target.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
                        socket.set_nonblocking(true)?;
                        socket_buffer(&socket)?;
                        Ok((socket, target))
                    }) {
                    Ok(socket) => output = Some(socket),
                    Err(e) => error = Some(e.to_string()),
                }
            }
            ctx.hub.set("logging", status(ctx, output.is_some(), error));
            applied = Some(desired);
        }
        }
        match input.recv(&mut record) {
            Ok(n) => {
                for byte in &mut record[..n] { if byte.is_ascii_control() { *byte = b' '; } }
                if !valid_record(&record[..n]) { continue; }
                retained.append(&record[..n])?;
                if let Some((socket, target)) = &output {
                    let _ = socket.send_to(&record[..n], target);
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {},
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[allow(unsafe_code)]
pub(crate) fn require_ram(path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: pointers are valid and statfs initializes the output on success.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 { return Err(io::Error::last_os_error()); }
    let stat = unsafe { stat.assume_init() };
    if stat.f_type as u32 != 0x01021994 && stat.f_type as u32 != 0x858458f6 {
        return Err(io::Error::other("logging directory is not tmpfs/ramfs; refusing flash fallback"));
    }
    Ok(())
}

#[allow(unsafe_code)]
pub(crate) fn require_ram_fd(file: &File) -> io::Result<()> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 { return Err(io::Error::last_os_error()); }
    let magic = unsafe { stat.assume_init() }.f_type as u32;
    if magic != 0x01021994 && magic != 0x858458f6 { return Err(io::Error::other("log descriptor is not on tmpfs/ramfs")); }
    Ok(())
}

type SocketSignature = (u64, u64, i64, i64);
struct SocketCleanup { path: std::path::PathBuf, _parent: File, dev: u64, inode: u64, owner: u32, changed: i64, changed_ns: i64, armed: bool }
fn socket_parent(path: &Path) -> io::Result<File> {
    let parent = path.parent().ok_or_else(|| io::Error::other("socket has no parent"))?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(parent)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.uid() != trusted_owner() || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0) {
        return Err(io::Error::other("socket parent is not trusted private/sticky storage"));
    }
    require_ram_fd(&file)?;
    Ok(file)
}
impl SocketCleanup {
    fn identity(path: &Path) -> io::Result<Self> {
        let parent = socket_parent(path)?;
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() || metadata.uid() != trusted_owner() || metadata.nlink() != 1 {
            return Err(io::Error::other("logging socket is not an owned single-link socket"));
        }
        Ok(Self { path: path.to_owned(), _parent: parent, dev: metadata.dev(), inode: metadata.ino(), owner: metadata.uid(), changed: metadata.ctime(), changed_ns: metadata.ctime_nsec(), armed: false })
    }
    fn signature(&self) -> SocketSignature { (self.dev, self.inode, self.changed, self.changed_ns) }
    fn bound(path: &Path) -> io::Result<Self> {
        let original = Self::identity(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let mut guard = Self::identity(path)?;
        if (original.dev, original.inode) != (guard.dev, guard.inode) { return Err(io::Error::other("bound socket identity changed")); }
        guard.armed = true;
        Ok(guard)
    }
    fn remove_same(&self) -> io::Result<bool> {
        match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == self.owner && metadata.mode() & 0o7777 == 0o600 && metadata.nlink() == 1 && (metadata.dev(), metadata.ino(), metadata.ctime(), metadata.ctime_nsec()) == self.signature() => { std::fs::remove_file(&self.path)?; Ok(true) },
            Ok(_) => Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        }
    }
}
impl Drop for SocketCleanup {
    fn drop(&mut self) { if self.armed { let _ = self.remove_same(); } }
}
fn read_socket_marker(lock: &mut File) -> io::Result<Option<SocketSignature>> {
    use std::io::{Read, Seek, SeekFrom};
    lock.seek(SeekFrom::Start(0))?;
    let mut bytes = [0u8; 128];
    let n = lock.read(&mut bytes)?;
    if n == 0 { return Ok(None); }
    let invalid = || io::Error::other("unrecognized collector ownership marker");
    let text = std::str::from_utf8(&bytes[..n]).map_err(|_| invalid())?;
    let mut fields = text.split_ascii_whitespace();
    if fields.next() != Some("lumatv-log-v1") { return Err(invalid()); }
    let dev = fields.next().ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
    let inode = fields.next().ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
    let changed = fields.next().ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
    let changed_ns = fields.next().ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
    if fields.next().is_some() { return Err(invalid()); }
    Ok(Some((dev, inode, changed, changed_ns)))
}
fn write_socket_marker(lock: &mut File, identity: &SocketCleanup) -> io::Result<()> {
    use std::io::{Seek, SeekFrom};
    private_file(lock, 128)?;
    let (dev, inode, changed, changed_ns) = identity.signature();
    let marker = format!("lumatv-log-v1 {dev} {inode} {changed} {changed_ns}\n");
    lock.seek(SeekFrom::Start(0))?;
    lock.set_len(0)?;
    lock.write_all(marker.as_bytes())
}
fn prepare_socket(path: &Path, previous: Option<SocketSignature>) -> io::Result<()> {
    let _parent = socket_parent(path)?;
    let identity = match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.mode() & 0o7777 != 0o600 { return Err(io::Error::other("existing logging socket is not private")); }
            SocketCleanup::identity(path)?
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if previous != Some(identity.signature()) { return Err(io::Error::other("existing socket has no matching collector ownership marker")); }
    let probe = UnixDatagram::unbound()?;
    probe.set_nonblocking(true)?;
    match probe.connect(path) {
        Ok(()) => return Err(io::Error::other("logging socket is still active")),
        Err(error) if error.raw_os_error() == Some(libc::ECONNREFUSED) => {},
        Err(error) => return Err(error),
    }
    if !identity.remove_same()? { return Err(io::Error::other("logging socket identity changed")); }
    Ok(())
}

fn valid_record(record: &[u8]) -> bool {
    if record.first() != Some(&b'<') { return false; }
    let Some(end) = record.iter().position(|b| *b == b'>') else { return false; };
    if !(2..=4).contains(&end) { return false; }
    let Ok(pri) = std::str::from_utf8(&record[1..end]).unwrap_or("").parse::<u8>() else { return false; };
    if pri > 191 { return false; }
    let tag = &record[end+1..];
    let Some(colon) = tag.iter().position(|b| *b == b':') else { return false; };
    colon > 0 && colon <= 64 && tag[..colon].iter().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(b))
}

/// libc DNS has no cancellation; isolate it in one child with a hard deadline.
fn resolve_bounded(host: &str, port: u16, stop: &crate::control::Stop) -> io::Result<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() { return Ok(SocketAddr::new(ip, port)); }
    if !valid_host(host) { return Err(io::Error::other("invalid server address")); }
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe()?).args(["__logging-resolve", host, &port.to_string()])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err(io::Error::other("server name could not be resolved")),
            Ok(None) if !stop.is_set() && std::time::Instant::now() < deadline => { std::thread::sleep(Duration::from_millis(20)); },
            result => {
                let _ = child.kill(); let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "server name lookup timed out or stopped")));
            }
        }
    }
    use std::io::Read;
    let mut bytes = [0u8; 64];
    let n = child.stdout.take().ok_or_else(|| io::Error::other("no resolver output"))?.read(&mut bytes)?;
    std::str::from_utf8(&bytes[..n]).ok().and_then(|s| s.trim().parse().ok()).ok_or_else(|| io::Error::other("invalid resolver output"))
}

/// Internal child entry; called before any service or logger is started.
pub fn resolver_entry(host: &str, port: &str) -> i32 {
    use std::net::ToSocketAddrs;
    if !valid_host(host) { return 2; }
    let Ok(port) = port.parse::<u16>() else { return 2; };
    match (host, port).to_socket_addrs().ok().and_then(|mut found| found.next()) {
        Some(address) => { println!("{address}"); 0 },
        None => 1,
    }
}

#[allow(unsafe_code)]
pub(crate) fn socket_buffer(socket: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let bytes: libc::c_int = 8192;
    for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        // SAFETY: descriptor and buffer pointer are valid for the call. Linux doubles this fixed request for bookkeeping.
        if unsafe { libc::setsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, option, (&bytes as *const libc::c_int).cast(), std::mem::size_of_val(&bytes) as libc::socklen_t) } != 0 { return Err(io::Error::last_os_error()); }
    }
    Ok(())
}


/// Local crash readers only: a bounded tail of the central RAM window, exact producer tag.
pub fn tail(tag: &str, lines: usize) -> Vec<String> {
    tail_at(Path::new(DIRECTORY), tag, lines)
}
fn tail_at(dir: &Path, tag: &str, lines: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut result = std::collections::VecDeque::with_capacity(lines.min(512));
    let limit = lines.min(512);
    if limit == 0 { return Vec::new(); }
    let Ok(directory) = LogDirectory::open(dir, false) else { return Vec::new(); };
    let mut bytes = [0u8; 32768];
    for name in [c"previous.log", c"current.log"] {
        let Ok(Some(mut file)) = directory.optional_log(name) else { continue; };
        let length = file.metadata().map_or(0, |meta| meta.len());
        let start = length.saturating_sub(bytes.len() as u64);
        if file.seek(SeekFrom::Start(start)).is_err() { continue; }
        let Ok(n) = file.read(&mut bytes) else { continue; };
        let text = String::from_utf8_lossy(&bytes[..n]);
        for (index, line) in text.lines().enumerate() {
            if index == 0 && start > 0 { continue; }
            let Some((_, record)) = line.split_once('>') else { continue; };
            if record.split_once(": ").is_some_and(|(source, _)| source == tag) {
                if result.len() == limit { result.pop_front(); }
                result.push_back(line.to_owned());
            }
        }
    }
    result.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ram_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir_in("/dev/shm").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
    #[test]
    fn exact_rotation_boundary_counts_header_and_newlines_inside_each_file_limit() {
        let dir = ram_fixture();
        let mut retained = Retained::open(dir.path()).unwrap();
        let bytes = [b'x'; RECORD_LIMIT];
        for _ in 0..3 {
            while retained.bytes < FILE_LIMIT {
                let remaining = FILE_LIMIT - retained.bytes;
                retained.append(&bytes[..(remaining - 1).min(RECORD_LIMIT as u64) as usize]).unwrap();
            }
            assert_eq!(std::fs::metadata(dir.path().join("current.log")).unwrap().len(), FILE_LIMIT);
            retained.append(b"x").unwrap();
            assert_eq!(std::fs::metadata(dir.path().join("previous.log")).unwrap().len(), FILE_LIMIT);
            assert_eq!(std::fs::metadata(dir.path().join("current.log")).unwrap().len(), FILE_HEADER.len() as u64 + 2);
            let total: u64 = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().metadata().unwrap().len()).sum();
            assert!(total <= RAM_LIMIT);
        }
    }

    #[test]
    fn hosts_are_not_urls_or_ports() {
        for host in ["127.0.0.1", "::1", "logs.example", "collector-1"] { assert!(valid_host(host)); }
        for host in ["", "https://logs", "logs:514", "-logs", "a..b", "a/b"] { assert!(!valid_host(host)); }
    }
    #[test]
    fn settings_defaults_and_rejected_updates_preserve_working_values() {
        use std::sync::Arc;
        use settings_schema::Value;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::settings::SettingsStore::new(dir.path().join("settings"), Arc::new(crate::hub::Hub::new(1)));
        store.register_builtin().unwrap();
        assert_eq!(store.flag("base.logging", "enabled"), Some(false));
        assert_eq!(store.text("base.logging", "host").as_deref(), Some(""));
        assert_eq!(store.integer("base.logging", "port"), Some(514));
        let old = store.describe("base.logging").unwrap();
        for changes in [vec![change("enabled", Value::Bool(true))], vec![change("port", Value::Integer(0))], vec![change("port", Value::Integer(65536))], vec![change("host", Value::Text("collector:514".into()))]] {
            assert!(store.apply("base.logging", changes, None).is_err());
            assert_eq!(store.describe("base.logging").unwrap(), old);
        }
    }
    #[test]
    fn retained_payload_is_bounded_even_for_oversized_records() {
        let dir = ram_fixture();
        let mut retained = Retained::open(dir.path()).unwrap();
        let payload = [b'x'; RECORD_LIMIT * 2];
        for _ in 0..2000 { retained.append(&payload).unwrap(); }
        let bytes: u64 = std::fs::read_dir(dir.path()).unwrap().map(|f| f.unwrap().metadata().unwrap().len()).sum();
        assert!(bytes <= RAM_LIMIT);
    }

    fn change(key: &str, value: settings_schema::Value) -> settings_schema::SettingChange {
        settings_schema::SettingChange { key: key.parse().unwrap(), value }
    }

    #[test]
    fn actual_host_udp_reroutes_and_stays_bounded_without_a_collector() {
        use std::sync::Arc;
        use settings_schema::Value;
        let dir = ram_fixture();
        let logs = dir.path().join("logs");
        let socket_path = dir.path().join("producer.sock");
        let hub = Arc::new(crate::hub::Hub::new(1));
        let settings = Arc::new(crate::settings::SettingsStore::new(dir.path().join("settings"), hub.clone()));
        settings.register_builtin().unwrap();
        let first = UdpSocket::bind("127.0.0.1:0").unwrap();
        let second = UdpSocket::bind("127.0.0.1:0").unwrap();
        first.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        second.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        settings.apply("base.logging", vec![change("enabled", Value::Bool(true)), change("host", Value::Text("127.0.0.1".into())), change("port", Value::Integer(first.local_addr().unwrap().port().into()))], None).unwrap();
        let ctx = Ctx { config: Arc::new(crate::config::Config::default()), paths: crate::config::Paths::new(dir.path()), hub: hub.clone(), settings: settings.clone(), stop: crate::control::Stop::new() };
        let worker_ctx = ctx.clone();
        let worker_logs = logs.clone();
        let worker_path = socket_path.clone();
        let worker = std::thread::Builder::new().stack_size(128 * 1024).spawn(move || run_at(&worker_ctx, &worker_logs, &worker_path)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while hub.get("logging").is_none() { assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
        let producer = UnixDatagram::unbound().unwrap();
        producer.set_nonblocking(true).unwrap();
        let mut received = [0u8; RECORD_LIMIT];
        producer.send_to(b"<14>lumatv-helper: host-smoke-first", &socket_path).unwrap();
        let n = first.recv(&mut received).unwrap();
        assert_eq!(&received[..n], b"<14>lumatv-helper: host-smoke-first");
        settings.apply("base.logging", vec![change("port", Value::Integer(second.local_addr().unwrap().port().into()))], None).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while hub.get("logging").unwrap()["server"]["port"] != second.local_addr().unwrap().port() { assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
        producer.send_to(b"<14>lumatv-helper: host-smoke-second", &socket_path).unwrap();
        let n = second.recv(&mut received).unwrap();
        assert_eq!(&received[..n], b"<14>lumatv-helper: host-smoke-second");
        let previous = settings.describe("base.logging").unwrap();
        assert!(settings.apply("base.logging", vec![change("host", Value::Text("https://bad".into()))], None).is_err());
        assert_eq!(settings.describe("base.logging").unwrap(), previous);
        drop(second);
        let mut payload = [b'x'; RECORD_LIMIT];
        payload[..19].copy_from_slice(b"<14>lumatv-helper: ");
        let started = std::time::Instant::now();
        for _ in 0..20000 { let _ = producer.send_to(&payload, &socket_path); }
        assert!(started.elapsed() < Duration::from_secs(3));
        ctx.stop.request();
        worker.join().unwrap().unwrap();
        let bytes: u64 = ["current.log", "previous.log"].iter().map(|name| std::fs::metadata(logs.join(name)).map_or(0, |m| m.len())).sum();
        assert!(bytes > 0 && bytes <= RAM_LIMIT);
        assert!(!socket_path.exists());
        println!("loopback tagged delivery and target change observed; retained bytes={bytes}; unavailable collector flood returned in {:?}", started.elapsed());
        for _ in 0..2 {
            let target = UdpSocket::bind("127.0.0.1:0").unwrap();
            target.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            settings.apply("base.logging", vec![change("port", Value::Integer(target.local_addr().unwrap().port().into()))], None).unwrap();
            let restarted = Ctx { stop: crate::control::Stop::new(), ..ctx.clone() };
            let worker_ctx = restarted.clone();
            let worker_logs = logs.clone();
            let worker_path = socket_path.clone();
            let worker = std::thread::Builder::new().stack_size(128 * 1024).spawn(move || run_at(&worker_ctx, &worker_logs, &worker_path)).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while !socket_path.exists() { assert!(std::time::Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
            producer.send_to(b"<14>lumatv-helper: host-smoke-restart", &socket_path).unwrap();
            let n = target.recv(&mut received).unwrap();
            assert_eq!(&received[..n], b"<14>lumatv-helper: host-smoke-restart");
            restarted.stop.request();
            worker.join().unwrap().unwrap();
            assert!(!socket_path.exists());
            for name in ["current.log", "previous.log"] { assert!(std::fs::metadata(logs.join(name)).map_or(0, |metadata| metadata.len()) <= FILE_LIMIT); }
        }
        println!("two complete collector restarts retained private owned logs and delivered tagged UDP messages");
    }

    fn private_fixture(path: &Path, text: &[u8]) {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).unwrap();
        file.write_all(text).unwrap();
    }
    #[allow(unsafe_code)]
    fn fixture_owner(path: &Path, owner: u32) {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::lchown(path.as_ptr(), owner, !0) }, 0);
    }
    #[test]
    fn private_unrelated_files_are_not_adopted_and_header_is_not_a_log_record() {
        let dir = ram_fixture();
        let current = dir.path().join("current.log");
        private_fixture(&current, b"unrelated private user data");
        assert!(Retained::open(dir.path()).is_err());
        assert_eq!(std::fs::read(&current).unwrap(), b"unrelated private user data");
        std::fs::remove_file(&current).unwrap();
        let previous = dir.path().join("previous.log");
        private_fixture(&previous, b"unrelated previous data");
        assert!(Retained::open(dir.path()).is_err());
        assert!(!current.exists());
        assert_eq!(std::fs::read(&previous).unwrap(), b"unrelated previous data");
        std::fs::remove_file(&previous).unwrap();
        let mut retained = Retained::open(dir.path()).unwrap();
        let first = b"<14>lumatv-helper: owned record";
        let second = b"<14>lumatv-other: ignored record";
        retained.append(first).unwrap();
        retained.append(second).unwrap();
        assert_eq!(tail_at(dir.path(), "lumatv-helper", 30), vec!["<14>lumatv-helper: owned record"]);
        assert_eq!(std::fs::metadata(current).unwrap().len(), (FILE_HEADER.len() + first.len() + second.len() + 2) as u64);
    }

    #[test]
    fn rejects_foreign_owned_directory_and_file_without_changing_them() {
        if trusted_owner() != 0 { eprintln!("foreign-owner fixture requires root; not exercised on this host"); return; }
        let dir = ram_fixture();
        let log_dir = dir.path().join("foreign");
        std::fs::DirBuilder::new().mode(0o700).create(&log_dir).unwrap();
        private_fixture(&log_dir.join("user-data"), b"preserve");
        fixture_owner(&log_dir, 65534);
        assert!(LogDirectory::open(&log_dir, true).is_err());
        assert_eq!(std::fs::metadata(&log_dir).unwrap().uid(), 65534);
        assert_eq!(std::fs::read(log_dir.join("user-data")).unwrap(), b"preserve");
        fixture_owner(&log_dir, trusted_owner());
        private_fixture(&log_dir.join("current.log"), b"foreign file");
        fixture_owner(&log_dir.join("current.log"), 65534);
        assert!(Retained::open(&log_dir).is_err());
        assert_eq!(std::fs::metadata(log_dir.join("current.log")).unwrap().uid(), 65534);
        assert_eq!(std::fs::read(log_dir.join("current.log")).unwrap(), b"foreign file");
    }
    #[test]
    fn rejects_public_modes_symlinks_and_hardlinks_without_touching_user_files() {
        use std::os::unix::fs::symlink;
        let dir = ram_fixture();
        let logs = dir.path().join("logs");
        std::fs::DirBuilder::new().mode(0o755).create(&logs).unwrap();
        assert!(Retained::open(&logs).is_err());
        assert_eq!(std::fs::metadata(&logs).unwrap().mode() & 0o777, 0o755);
        std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o700)).unwrap();
        let outside = dir.path().join("user-data");
        private_fixture(&outside, b"private user data");
        symlink(&outside, logs.join("current.log")).unwrap();
        assert!(Retained::open(&logs).is_err());
        std::fs::remove_file(logs.join("current.log")).unwrap();
        std::fs::hard_link(&outside, logs.join("current.log")).unwrap();
        assert!(Retained::open(&logs).is_err());
        assert_eq!(std::fs::metadata(&outside).unwrap().nlink(), 2);
        std::fs::remove_file(logs.join("current.log")).unwrap();
        private_fixture(&logs.join("current.log"), b"public current");
        std::fs::set_permissions(logs.join("current.log"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Retained::open(&logs).is_err());
        assert_eq!(std::fs::read(logs.join("current.log")).unwrap(), b"public current");
        assert_eq!(std::fs::read(&outside).unwrap(), b"private user data");
        symlink(&logs, dir.path().join("linked-dir")).unwrap();
        assert!(LogDirectory::open(&dir.path().join("linked-dir"), true).is_err());
    }
    #[test]
    fn rotation_refuses_unrelated_previous_and_changed_current_identity() {
        let dir = ram_fixture();
        let mut retained = Retained::open(dir.path()).unwrap();
        private_fixture(&dir.path().join("previous.log"), b"unrelated previous");
        std::fs::set_permissions(dir.path().join("previous.log"), std::fs::Permissions::from_mode(0o644)).unwrap();
        retained.bytes = FILE_LIMIT;
        assert!(retained.append(b"rotate").is_err());
        assert_eq!(std::fs::read(dir.path().join("previous.log")).unwrap(), b"unrelated previous");
        std::fs::set_permissions(dir.path().join("previous.log"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::rename(dir.path().join("current.log"), dir.path().join("old-current")).unwrap();
        private_fixture(&dir.path().join("current.log"), b"replacement current");
        assert!(retained.append(b"rotate").is_err());
        assert_eq!(std::fs::read(dir.path().join("current.log")).unwrap(), b"replacement current");
        assert_eq!(std::fs::read(dir.path().join("previous.log")).unwrap(), b"unrelated previous");
    }
    #[test]
    fn socket_startup_preserves_unmarked_foreign_and_live_sockets() {
        let dir = ram_fixture();
        let path = dir.path().join("collector.sock");
        let socket = UnixDatagram::bind(&path).unwrap();
        let mut guard = SocketCleanup::bound(&path).unwrap();
        let signature = guard.signature();
        assert!(prepare_socket(&path, None).is_err());
        assert!(prepare_socket(&path, Some(signature)).is_err());
        assert!(path.exists());
        drop(socket);
        guard.armed = false;
        drop(guard);
        assert!(prepare_socket(&path, None).is_err());
        assert!(path.exists());
        prepare_socket(&path, Some(signature)).unwrap();
        assert!(!path.exists());
        let replacement = UnixDatagram::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        if trusted_owner() == 0 {
            fixture_owner(&path, 65534);
            assert!(prepare_socket(&path, Some(signature)).is_err());
            assert_eq!(std::fs::symlink_metadata(&path).unwrap().uid(), 65534);
        } else { eprintln!("foreign socket owner requires root; not exercised on this host"); }
        drop(replacement);
    }
    #[test]
    fn socket_cleanup_leaves_replacement_socket_and_regular_file_untouched() {
        let dir = ram_fixture();
        let path = dir.path().join("collector.sock");
        let original = UnixDatagram::bind(&path).unwrap();
        let guard = SocketCleanup::bound(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let replacement = UnixDatagram::bind(&path).unwrap();
        replacement.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(guard);
        UnixDatagram::unbound().unwrap().send_to(b"replacement survives", &path).unwrap();
        let mut bytes = [0u8; 64];
        let n = replacement.recv(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"replacement survives");
        let guard = SocketCleanup::bound(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        private_fixture(&path, b"replacement user file");
        drop(guard);
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement user file");
        drop(original);
    }
    #[test]
    fn socket_parent_and_lock_marker_are_validated_without_adoption() {
        let dir = ram_fixture();
        let parent = dir.path().join("public");
        std::fs::DirBuilder::new().mode(0o777).create(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(prepare_socket(&parent.join("log.sock"), None).is_err());
        assert!(!parent.join("log.sock").exists());
        let directory = LogDirectory::open(dir.path(), false).unwrap();
        let mut lock = directory.lock().unwrap();
        assert!(directory.lock().is_err());
        assert_eq!(read_socket_marker(&mut lock).unwrap(), None);
        let path = dir.path().join("owned.sock");
        let input = UnixDatagram::bind(&path).unwrap();
        let mut guard = SocketCleanup::bound(&path).unwrap();
        write_socket_marker(&mut lock, &guard).unwrap();
        assert_eq!(read_socket_marker(&mut lock).unwrap(), Some(guard.signature()));
        guard.armed = false;
        drop(input);
        let signature = guard.signature();
        drop(guard);
        drop(lock);
        let mut lock = directory.lock().unwrap();
        prepare_socket(&path, read_socket_marker(&mut lock).unwrap()).unwrap();
        assert!(!path.exists());
        let input = UnixDatagram::bind(&path).unwrap();
        let guard = SocketCleanup::bound(&path).unwrap();
        assert_ne!(guard.signature(), signature);
        write_socket_marker(&mut lock, &guard).unwrap();
        drop(guard);
        drop(input);
        assert!(!path.exists());
    }

    #[test]
    fn flash_fallback_is_refused() {
        assert!(require_ram(Path::new("/proc")).is_err());
    }
}
