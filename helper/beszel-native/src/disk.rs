//! Disk usage and I/O for the root filesystem (`FILESYSTEM`) and `EXTRA_FILESYSTEMS`, discovered and
//! measured as the Go agent does it (`agent/disk.go` with gopsutil's mountinfo and diskstats readers).

use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::path::Path;
use std::time::Instant;

use crate::slog;
use crate::system::{FsStats, Stats, gigabytes, megabytes, two_decimals};

const EXTRA_FILESYSTEMS_DIR: &str = "/extra-filesystems";

#[derive(Debug, Clone, PartialEq)]
pub struct Partition {
    pub device: String,
    pub mountpoint: String,
}

/// gopsutil's `parseFieldsOnMountinfo` for every mount, bind mounts included.
pub fn parse_mountinfo(text: &str) -> Result<Vec<Partition>, String> {
    let mut partitions = Vec::new();
    let mut first_device: HashMap<&str, String> = HashMap::new();
    for line in text.lines() {
        let invalid = || format!("found invalid mountinfo line: {line}");
        let (left, right) = line.split_once(" - ").ok_or_else(invalid)?;
        let fields: Vec<&str> = left.split_whitespace().collect();
        let source: Vec<&str> = right.split_whitespace().collect();
        if fields.len() < 5 || source.len() < 2 {
            return Err(invalid());
        }
        let (id, root, mountpoint, source) = (fields[2], fields[3], fields[4], source[1]);
        // Virtual filesystems name themselves in the root field; a bind mount reuses the device
        // its block device was first seen with.
        let mut device = if source.starts_with('/') || root == "/" { source } else { root }.to_owned();
        match first_device.get(id) {
            Some(first) => device.clone_from(first),
            None => {
                first_device.insert(id, device.clone());
            }
        }
        if device.starts_with("/dev/mapper/")
            && let Ok(resolved) = std::fs::canonicalize(&device)
        {
            device = resolved.to_string_lossy().into_owned();
        }
        if device == "/dev/root"
            && let Ok(target) = std::fs::read_link(format!("/sys/dev/block/{id}"))
        {
            device = format!("/dev/{}", go_base(&target.to_string_lossy()));
        }
        partitions.push(Partition { device, mountpoint: unescape(mountpoint) });
    }
    Ok(partitions)
}

/// Mount points escape spaces, tabs, newlines and backslashes as three octal digits.
fn unescape(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).filter(|d| bytes[i] == b'\\' && d.iter().all(|b| (b'0'..=b'7').contains(b)));
        match octal {
            Some(digits) => {
                let value = digits.iter().fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
                let Ok(byte) = u8::try_from(value) else { return path.to_owned() };
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| path.to_owned())
}

/// Go's `filepath.Base`.
fn go_base(path: &str) -> &str {
    if path.is_empty() {
        return ".";
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/";
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Go's `filepath.Dir` for the absolute, clean paths mountinfo lists.
fn go_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(at) => &path[..at],
        None => ".",
    }
}

/// `device__name`: a device or path and an optional display name.
fn parse_entry(entry: &str) -> (&str, &str) {
    let entry = entry.trim();
    match entry.split_once("__") {
        Some((device, name)) => (device.trim(), name.trim()),
        None => (entry, ""),
    }
}

fn normalize_device(value: &str) -> &str {
    match go_base(value.trim()) {
        "." => "",
        base => base,
    }
}

fn prefix_related(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && a != b && (a.starts_with(b) || b.starts_with(a))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counters {
    pub name: String,
    /// The device-mapper name, used only to match configured devices.
    pub label: String,
    pub read_count: u64,
    pub read_bytes: u64,
    pub read_time: u64,
    pub write_count: u64,
    pub write_bytes: u64,
    pub write_time: u64,
    pub io_time: u64,
    pub weighted_io: u64,
}

/// gopsutil's diskstats reader: devices with no activity at all are left out.
pub fn parse_diskstats(text: &str) -> Result<Vec<Counters>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 14 {
            continue;
        }
        let mut values = [0u64; 11];
        for (value, field) in values.iter_mut().zip(&fields[3..14]) {
            *value = field.parse().map_err(|err| format!("diskstats {field:?}: {err}"))?;
        }
        if values == [0; 11] {
            continue;
        }
        let [reads, _, read_sectors, read_time, writes, _, write_sectors, write_time, _, io_time, weighted_io] = values;
        out.push(Counters {
            name: fields[2].to_owned(),
            label: String::new(),
            read_count: reads,
            read_bytes: read_sectors.wrapping_mul(512),
            read_time,
            write_count: writes,
            write_bytes: write_sectors.wrapping_mul(512),
            write_time,
            io_time,
            weighted_io,
        });
    }
    Ok(out)
}

fn read_counters() -> Result<Vec<Counters>, String> {
    parse_diskstats(&std::fs::read_to_string("/proc/diskstats").map_err(|err| err.to_string())?)
}

/// gopsutil's `disk.Usage`: blocks counted in `f_bsize`.
#[derive(Debug, Clone, Copy)]
struct Usage {
    total: u64,
    used: u64,
    free: u64,
}

impl Usage {
    fn used_percent(&self) -> f64 {
        let known = self.used.wrapping_add(self.free);
        if known == 0 { 0.0 } else { self.used as f64 / known as f64 * 100.0 }
    }
}

fn usage(path: &str) -> io::Result<Usage> {
    let path = CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `path` is a valid C string and `stat` a writable statfs.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = stat.f_bsize as u64;
    Ok(Usage {
        total: (stat.f_blocks as u64).wrapping_mul(block),
        used: (stat.f_blocks as u64).wrapping_sub(stat.f_bfree as u64).wrapping_mul(block),
        free: (stat.f_bavail as u64).wrapping_mul(block),
    })
}

/// Two readings a few milliseconds apart drift; within this they are the same filesystem.
const SAME_USAGE_TOLERANCE: u64 = 16 << 20;

fn same_usage(a: &Usage, b: &Usage) -> bool {
    a.total != 0 && b.total != 0 && a.total.abs_diff(b.total) <= SAME_USAGE_TOLERANCE && a.used.abs_diff(b.used) <= SAME_USAGE_TOLERANCE
}

/// The counters one interval last measured from.
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    read_bytes: u64,
    write_bytes: u64,
    read_time: u64,
    write_time: u64,
    io_time: u64,
    weighted_io: u64,
    read_count: u64,
    write_count: u64,
    at: Instant,
}

impl Snapshot {
    fn new(d: &Counters, at: Instant) -> Snapshot {
        Snapshot {
            read_bytes: d.read_bytes,
            write_bytes: d.write_bytes,
            read_time: d.read_time,
            write_time: d.write_time,
            io_time: d.io_time,
            weighted_io: d.weighted_io,
            read_count: d.read_count,
            write_count: d.write_count,
            at,
        }
    }
}

/// The time counters are 32-bit in `/proc/diskstats` and wrap; a busy disk wraps the weighted one in days.
fn io_time_delta(current: u64, previous: u64) -> u64 {
    if current >= previous {
        current - previous
    } else if previous <= u64::from(u32::MAX) {
        current + ((1u64 << 32) - previous)
    } else {
        0
    }
}

#[derive(Debug)]
struct Fs {
    /// The diskstats device the filesystem's I/O is read from, or its name when it has none.
    key: String,
    root: bool,
    mountpoint: String,
    name: String,
    stats: FsStats,
}

pub struct Config<'a> {
    /// `FILESYSTEM`.
    pub filesystem: &'a str,
    /// `EXTRA_FILESYSTEMS`, when set.
    pub extra: Option<&'a str>,
}

pub struct Disks {
    list: Vec<Fs>,
    /// Devices with diskstats entries.
    io_names: Vec<String>,
    prev: HashMap<u16, HashMap<String, Snapshot>>,
    /// The latest counters of each device; a new interval measures its first sample from them.
    baseline: HashMap<String, Snapshot>,
}

/// One run of discovery, as `diskDiscovery` holds it.
struct Discovery<'a> {
    disks: &'a mut Disks,
    partitions: &'a [Partition],
    counters: &'a [Counters],
    filesystem: &'a str,
    root_mountpoint: String,
}

impl Discovery<'_> {
    fn has_io(&self, key: &str) -> bool {
        self.counters.iter().any(|d| d.name == key)
    }

    /// An exact device or device-mapper name, else the busiest device whose name is a prefix of the
    /// other's (`nvme0n1p2` and `nvme0n1`).
    fn find_io_device(&self, filesystem: &str) -> Option<String> {
        let wanted = normalize_device(filesystem);
        if wanted.is_empty() {
            return None;
        }
        let mut best: Option<&Counters> = None;
        for d in self.counters {
            let name = normalize_device(&d.name);
            let label = (!d.label.is_empty()).then(|| normalize_device(&d.label));
            if name == wanted || label == Some(wanted) {
                return Some(d.name.clone());
            }
            if prefix_related(name, wanted) || label.is_some_and(|label| prefix_related(label, wanted)) {
                best = Some(match best {
                    Some(b) if !busier(d, b) => b,
                    _ => d,
                });
            }
        }
        let best = best?;
        slog::info("Using disk I/O fallback", &[("requested", &wanted), ("selected", &best.name)]);
        Some(best.name.clone())
    }

    /// `registerFilesystemStats`: the diskstats key for a filesystem, `None` when already tracked.
    fn register(&self, device: &str, mountpoint: &str, root: bool) -> Option<String> {
        let mut key = go_base(device).to_owned();
        let mut resolved_key = String::new();
        // `/dev/vg/lv` names neither its diskstats entry (`dm-N`) nor its label; the link target does.
        if device.starts_with('/')
            && let Ok(resolved) = std::fs::canonicalize(device)
        {
            let base = go_base(&resolved.to_string_lossy()).to_owned();
            if base != key {
                resolved_key = base;
            }
        }
        if !self.has_io(&key) {
            if root {
                if let Some(found) = self.find_io_device(&key).or_else(|| self.find_io_device(&resolved_key)) {
                    key = found;
                } else if !self.filesystem.is_empty()
                    && let Some(found) = self.find_io_device(self.filesystem)
                {
                    key = found;
                }
                if !self.has_io(&key) {
                    slog::warn("Root I/O unmapped; set FILESYSTEM", &[("device", &device), ("mountpoint", &mountpoint)]);
                }
            } else {
                if mountpoint.starts_with(EXTRA_FILESYSTEMS_DIR) {
                    let (folder_device, _) = parse_entry(go_base(mountpoint));
                    if let Some(found) = self.find_io_device(folder_device) {
                        key = found;
                    }
                }
                if !self.has_io(&key)
                    && let Some(found) = self.find_io_device(&key).or_else(|| self.find_io_device(&resolved_key))
                {
                    key = found;
                }
            }
        }
        (!self.disks.list.iter().any(|fs| fs.key == key)).then_some(key)
    }

    fn add(&mut self, device: &str, mountpoint: &str, root: bool, name: &str) -> bool {
        let Some(key) = self.register(device, mountpoint, root) else { return false };
        let shown = if name.is_empty() { key.as_str() } else { name };
        slog::info(
            "Detected disk",
            &[("name", &shown), ("device", &device), ("mount", &mountpoint), ("io", &key), ("root", &root)],
        );
        self.disks.list.push(Fs { key, root, mountpoint: mountpoint.to_owned(), name: name.to_owned(), stats: FsStats::default() });
        true
    }

    fn set(&mut self, key: String, mountpoint: String, name: &str) {
        self.disks.list.retain(|fs| fs.key != key);
        self.disks.list.push(Fs { key, root: true, mountpoint, name: name.to_owned(), stats: FsStats::default() });
    }

    fn add_configured_root(&mut self, name: &str) -> bool {
        let filesystem = self.filesystem;
        if filesystem.is_empty() {
            return false;
        }
        if let Some(p) = self.partitions.iter().find(|p| matches_partition(filesystem, p)) {
            self.add(&p.device, &p.mountpoint, true, name);
            return true;
        }
        // ZFS lists datasets, not block devices, as partitions.
        if let Some(key) = self.find_io_device(filesystem) {
            let mountpoint = self.root_mountpoint.clone();
            self.set(key, mountpoint, name);
            return true;
        }
        slog::warn("Partition details not found", &[("filesystem", &filesystem)]);
        false
    }

    fn add_configured_extra(&mut self, entry: &str) {
        let (filesystem, name) = parse_entry(entry);
        if let Some(p) = self.partitions.iter().find(|p| p.device.ends_with(filesystem) || p.mountpoint == filesystem) {
            self.add(&p.device, &p.mountpoint, false, name);
            return;
        }
        match usage(filesystem) {
            Ok(_) => {
                self.add(go_base(filesystem), filesystem, false, name);
            }
            Err(err) => slog::error("Invalid filesystem", &[("name", &filesystem), ("err", &err)]),
        }
    }

    fn add_partition_root(&mut self, device: &str, mountpoint: &str) -> bool {
        let Some(key) = self.find_io_device(device) else { return false };
        // The root device may already be tracked as an extra filesystem; it becomes the root.
        if let Some(fs) = self.disks.list.iter_mut().find(|fs| fs.key == key) {
            fs.root = true;
            mountpoint.clone_into(&mut fs.mountpoint);
            return true;
        }
        self.add(&key, mountpoint, true, "")
    }

    fn add_last_resort_root(&mut self) {
        let busiest = self.counters.iter().fold(None, |best: Option<&Counters>, d| match best {
            Some(b) if !busier(d, b) => Some(b),
            _ => Some(d),
        });
        let key = match busiest {
            Some(d) => {
                slog::warn("Using most active device for root I/O; set FILESYSTEM to override", &[("device", &d.name)]);
                d.name.clone()
            }
            None => {
                let mut key = go_base(&self.root_mountpoint).to_owned();
                if self.disks.list.iter().any(|fs| fs.key == key) {
                    key = "root".to_owned();
                }
                slog::warn("Root I/O device not detected; set FILESYSTEM to override", &[]);
                key
            }
        };
        let mountpoint = self.root_mountpoint.clone();
        self.set(key, mountpoint, "");
    }
}

/// `findIoDevice` and `mostActiveIoDevice` order: more bytes, then more operations, then the name.
fn busier(a: &Counters, b: &Counters) -> bool {
    let (a_bytes, b_bytes) = (a.read_bytes.wrapping_add(a.write_bytes), b.read_bytes.wrapping_add(b.write_bytes));
    let (a_ops, b_ops) = (a.read_count.wrapping_add(a.write_count), b.read_count.wrapping_add(b.write_count));
    a_bytes > b_bytes || (a_bytes == b_bytes && (a_ops > b_ops || (a_ops == b_ops && a.name < b.name)))
}

/// `FILESYSTEM` names a partition by mount point, device or a prefix-related device (`sda` for `sda2`).
fn matches_partition(filesystem: &str, p: &Partition) -> bool {
    let filesystem = filesystem.trim();
    if filesystem.is_empty() {
        return false;
    }
    if p.mountpoint == filesystem {
        return true;
    }
    let (wanted, device) = (normalize_device(filesystem), normalize_device(&p.device));
    !wanted.is_empty() && !device.is_empty() && (wanted == device || prefix_related(device, wanted))
}

/// `/sysroot` on image-based systems such as Silverblue, else `/`.
fn root_mountpoint() -> String {
    let immutable = std::fs::read_to_string("/etc/os-release").is_ok_and(|content| {
        (content.contains("fedora") && content.contains("silverblue"))
            || ["coreos", "flatcar", "rhel-atomic", "centos-atomic"].iter().any(|id| content.contains(id))
    });
    if (immutable || Path::new("/run/ostree").exists()) && Path::new("/sysroot").exists() {
        return "/sysroot".to_owned();
    }
    "/".to_owned()
}

fn device_label(name: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/block/{name}/dm/name")).map(|label| label.trim().to_owned()).unwrap_or_default()
}

impl Disks {
    pub fn discover(config: &Config) -> Disks {
        let (filesystem, root_name) = parse_entry(config.filesystem);
        let mountinfo = std::fs::read_to_string("/proc/1/mountinfo").or_else(|_| std::fs::read_to_string("/proc/self/mountinfo"));
        let partitions = mountinfo.map_err(|err| err.to_string()).and_then(|text| parse_mountinfo(&text)).unwrap_or_else(|err| {
            slog::error("Error getting disk partitions", &[("err", &err)]);
            Vec::new()
        });
        let mut counters = read_counters().unwrap_or_else(|err| {
            slog::error("Error getting diskstats", &[("err", &err)]);
            Vec::new()
        });
        counters.sort_by(|a, b| a.name.cmp(&b.name));
        for d in &mut counters {
            d.label = device_label(&d.name);
        }
        let mut disks = Disks { list: Vec::new(), io_names: Vec::new(), prev: HashMap::new(), baseline: HashMap::new() };
        let mut discovery = Discovery {
            disks: &mut disks,
            partitions: &partitions,
            counters: &counters,
            filesystem,
            root_mountpoint: root_mountpoint(),
        };

        let mut has_root = discovery.add_configured_root(root_name);
        if let Some(extra) = config.extra {
            for entry in extra.split(',') {
                discovery.add_configured_extra(entry);
            }
        }
        for p in &partitions {
            let docker_file = matches!(p.mountpoint.as_str(), "/etc/hosts" | "/etc/resolv.conf" | "/etc/hostname");
            if !has_root && (p.mountpoint == discovery.root_mountpoint || (docker_file && p.device.starts_with("/dev"))) {
                has_root = discovery.add_partition_root(&p.device, &p.mountpoint);
            }
            if go_dir(&p.mountpoint) == EXTRA_FILESYSTEMS_DIR {
                let (folder_device, name) = parse_entry(go_base(&p.mountpoint));
                let device = if p.device.trim().is_empty() { folder_device } else { p.device.trim() };
                discovery.add(device, &p.mountpoint, false, name);
            }
        }
        if let Ok(entries) = std::fs::read_dir(EXTRA_FILESYSTEMS_DIR) {
            let mut folders: Vec<String> = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            folders.sort();
            let known: Vec<String> = discovery.disks.list.iter().map(|fs| fs.mountpoint.clone()).collect();
            for folder in folders {
                let mountpoint = format!("{EXTRA_FILESYSTEMS_DIR}/{folder}");
                if !known.contains(&mountpoint) {
                    let (device, name) = parse_entry(&folder);
                    discovery.add(device, &mountpoint, false, name);
                }
            }
        }
        if !has_root {
            discovery.add_last_resort_root();
        }
        disks.prune_duplicates_of_root();
        disks.start_io(&counters, Instant::now());
        disks
    }

    /// Extra filesystems that report the root's usage are the root seen through another mount.
    fn prune_duplicates_of_root(&mut self) {
        let Some(root) = self.list.iter().find(|fs| fs.root) else { return };
        let Ok(root_usage) = usage(&root.mountpoint) else { return };
        self.list.retain(|fs| {
            let duplicate = !fs.root && usage(&fs.mountpoint).is_ok_and(|u| same_usage(&root_usage, &u));
            if duplicate {
                slog::info("Ignoring duplicate FS", &[("name", &fs.key), ("mount", &fs.mountpoint)]);
            }
            !duplicate
        });
    }

    fn start_io(&mut self, counters: &[Counters], now: Instant) {
        self.io_names.clear();
        for fs in &mut self.list {
            let Some(d) = counters.iter().find(|d| d.name == fs.key) else {
                slog::warn("Device not found in diskstats", &[("name", &fs.key)]);
                continue;
            };
            fs.stats.total_read = d.read_bytes;
            fs.stats.total_write = d.write_bytes;
            self.baseline.insert(fs.key.clone(), Snapshot::new(d, now));
            self.io_names.push(fs.key.clone());
        }
    }

    pub fn update_usage(&mut self, stats: &mut Stats) {
        for fs in &mut self.list {
            match usage(&fs.mountpoint) {
                Ok(u) => {
                    fs.stats.total = gigabytes(u.total);
                    fs.stats.used = gigabytes(u.used);
                    if fs.root {
                        stats.disk_total = fs.stats.total;
                        stats.disk_used = fs.stats.used;
                        stats.disk_pct = two_decimals(u.used_percent());
                    }
                }
                Err(err) => {
                    // Most likely unmounted.
                    slog::error("Error getting disk stats", &[("name", &fs.mountpoint), ("err", &err)]);
                    fs.stats.total = 0.0;
                    fs.stats.used = 0.0;
                    fs.stats.total_read = 0;
                    fs.stats.total_write = 0;
                }
            }
        }
    }

    pub fn update_io(&mut self, interval: u16, stats: &mut Stats) {
        let Ok(mut counters) = read_counters() else { return };
        if !self.io_names.is_empty() {
            counters.retain(|d| self.io_names.contains(&d.name));
        }
        self.measure_io(interval, stats, &counters, Instant::now());
    }

    fn measure_io(&mut self, interval: u16, stats: &mut Stats, counters: &[Counters], now: Instant) {
        for d in counters {
            if !self.list.iter().any(|fs| fs.key == d.name) {
                continue;
            }
            let prev_map = self.prev.entry(interval).or_default();
            let (prev, first) = match prev_map.get(&d.name) {
                Some(prev) => (*prev, false),
                None => (self.baseline.get(&d.name).copied().unwrap_or_else(|| Snapshot::new(d, now)), true),
            };
            prev_map.insert(d.name.clone(), Snapshot::new(d, now));
            let ms = now.saturating_duration_since(prev.at).as_millis() as u64;
            // A first sample right after start spans only the start's own I/O burst.
            if ms < 100 || (first && ms < u64::from(interval) / 2) {
                continue;
            }
            let read = d.read_bytes.wrapping_sub(prev.read_bytes).wrapping_mul(1000) / ms;
            let write = d.write_bytes.wrapping_sub(prev.write_bytes).wrapping_mul(1000) / ms;
            let (read_mb, write_mb) = (megabytes(read as f64), megabytes(write as f64));
            if read_mb > 50_000.0 || write_mb > 50_000.0 {
                slog::warn("Invalid disk I/O. Resetting.", &[("name", &d.name), ("read", &read_mb), ("write", &write_mb)]);
                self.start_io(counters, now);
                continue;
            }
            let read_time = io_time_delta(d.read_time, prev.read_time);
            let write_time = io_time_delta(d.write_time, prev.write_time);
            let percent = |delta: u64| two_decimals(delta as f64 / ms as f64 * 100.0);
            let await_ms = |time: u64, ops: u64| if ops > 0 { two_decimals(time as f64 / ops as f64) } else { 0.0 };
            let io_stats = [
                percent(read_time),
                percent(write_time),
                percent(io_time_delta(d.io_time, prev.io_time)),
                await_ms(read_time, d.read_count.wrapping_sub(prev.read_count)),
                await_ms(write_time, d.write_count.wrapping_sub(prev.write_count)),
                percent(io_time_delta(d.weighted_io, prev.weighted_io)),
            ];
            self.baseline.insert(d.name.clone(), Snapshot::new(d, now));
            let Some(fs) = self.list.iter_mut().find(|fs| fs.key == d.name) else { continue };
            fs.stats.total_read = d.read_bytes;
            fs.stats.total_write = d.write_bytes;
            fs.stats.read_bytes = read;
            fs.stats.write_bytes = write;
            fs.stats.io_stats = io_stats;
            if fs.root {
                stats.disk_io = [read, write];
                stats.disk_io_total = [d.read_bytes, d.write_bytes];
                stats.disk_io_stats = io_stats;
            }
        }
    }

    /// The root's display name (`FILESYSTEM=device__name`).
    pub fn root_name(&self) -> &str {
        self.list.iter().find(|fs| fs.root).map_or("", |fs| fs.name.as_str())
    }

    /// Extra filesystems with a known size, by display name.
    pub fn extra(&self) -> impl Iterator<Item = (&str, &FsStats)> {
        self.list
            .iter()
            .filter(|fs| !fs.root && fs.stats.total > 0.0)
            .map(|fs| (if fs.name.is_empty() { fs.key.as_str() } else { fs.name.as_str() }, &fs.stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reads_mounts_as_gopsutil() {
        let text = "20 1 179:58 / /var rw,relatime - ext4 /dev/mmcblk0p58 rw\n\
                    21 1 0:5 / /dev/shm rw - tmpfs tmpfs rw\n\
                    22 1 179:58 /data /mnt/my\\040data rw - ext4 /dev/mmcblk0p58 rw\n\
                    23 1 0:6 net:[4026] /run/netns rw - nsfs nsfs rw\n";
        let partitions = parse_mountinfo(text).unwrap();
        let pairs: Vec<(&str, &str)> = partitions.iter().map(|p| (p.device.as_str(), p.mountpoint.as_str())).collect();
        assert_eq!(
            pairs,
            [("/dev/mmcblk0p58", "/var"), ("tmpfs", "/dev/shm"), ("/dev/mmcblk0p58", "/mnt/my data"), ("net:[4026]", "/run/netns")]
        );
        assert!(parse_mountinfo("no separator\n").is_err());
    }

    #[test]
    fn reads_diskstats_as_gopsutil() {
        let text = " 179  0 mmcblk0 10 0 8 30 20 0 16 40 0 50 70 0 0 0 0\n   7  0 loop0 0 0 0 0 0 0 0 0 0 0 0\n";
        let counters = parse_diskstats(text).unwrap();
        assert_eq!(counters.len(), 1);
        let d = &counters[0];
        assert_eq!((d.read_bytes, d.write_bytes, d.read_time, d.io_time, d.weighted_io), (4096, 8192, 30, 50, 70));
        assert!(parse_diskstats(" 1 0 sda x 0 0 0 0 0 0 0 0 0 0\n").is_err());
    }

    fn device(name: &str, bytes: u64) -> Counters {
        Counters { name: name.into(), read_bytes: bytes, ..Counters::default() }
    }

    #[test]
    fn matches_configured_devices_like_the_go_agent() {
        let counters = [device("mmcblk0", 50), device("mmcblk0p58", 10), device("sda", 5)];
        let mut disks = Disks { list: Vec::new(), io_names: Vec::new(), prev: HashMap::new(), baseline: HashMap::new() };
        let discovery = Discovery { disks: &mut disks, partitions: &[], counters: &counters, filesystem: "", root_mountpoint: "/".into() };
        assert_eq!(discovery.find_io_device("/dev/mmcblk0p58").as_deref(), Some("mmcblk0p58"));
        // No exact entry: the busiest related device.
        assert_eq!(discovery.find_io_device("mmcblk0p1").as_deref(), Some("mmcblk0"));
        assert_eq!(discovery.find_io_device("nvme0n1"), None);

        let var = Partition { device: "/dev/mmcblk0p58".into(), mountpoint: "/var".into() };
        assert!(matches_partition("/var", &var));
        assert!(matches_partition("mmcblk0", &var));
        assert!(!matches_partition("sda", &var));
        assert_eq!(parse_entry(" /media__Apps "), ("/media", "Apps"));
        assert_eq!(unescape("/mnt/a\\040b\\134c"), "/mnt/a b\\c");
    }

    #[test]
    fn measures_io_per_interval() {
        let start = Instant::now();
        let mut disks = Disks {
            list: vec![Fs { key: "mmcblk0".into(), root: true, mountpoint: "/".into(), name: String::new(), stats: FsStats::default() }],
            io_names: vec!["mmcblk0".into()],
            prev: HashMap::new(),
            baseline: HashMap::new(),
        };
        let at = |read_bytes: u64, io_time: u64, read_count: u64, read_time: u64| Counters {
            name: "mmcblk0".into(),
            read_bytes,
            io_time,
            read_count,
            read_time,
            ..Counters::default()
        };
        disks.start_io(&[at(0, 0, 0, 0)], start);

        // The first sample of an interval must span half of it.
        let mut stats = Stats::default();
        disks.measure_io(1000, &mut stats, &[at(1000, 10, 1, 2)], start + Duration::from_millis(400));
        assert_eq!(stats.disk_io, [0, 0]);

        let mut stats = Stats::default();
        disks.measure_io(1000, &mut stats, &[at(3000, 510, 3, 12)], start + Duration::from_millis(1400));
        assert_eq!(stats.disk_io, [2000, 0]);
        assert_eq!(stats.disk_io_total, [3000, 0]);
        // 500 ms busy of 1000; 10 ms over two reads.
        assert_eq!(stats.disk_io_stats, [1.0, 0.0, 50.0, 5.0, 0.0, 0.0]);

        // A new interval measures from the latest sample of any interval.
        let mut stats = Stats::default();
        disks.measure_io(60_000, &mut stats, &[at(63_000, 510, 3, 12)], start + Duration::from_millis(31_400));
        assert_eq!(stats.disk_io, [2000, 0]);
        assert_eq!(io_time_delta(5, u64::from(u32::MAX) - 4), 10);
    }
}
