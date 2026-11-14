//! What the hub shows about the machine, and the data directory that keeps the agent's identity.

use std::ffi::CStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::system::{Details, two_decimals};

struct Uname {
    nodename: String,
    release: String,
    machine: String,
}

fn uname() -> Uname {
    // SAFETY: `uname` fills the zeroed struct with NUL-terminated strings.
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut uts) } != 0 {
        return Uname { nodename: String::new(), release: String::new(), machine: String::new() };
    }
    let field = |chars: &[libc::c_char]| unsafe { CStr::from_ptr(chars.as_ptr()) }.to_string_lossy().into_owned();
    Uname { nodename: field(&uts.nodename), release: field(&uts.release), machine: field(&uts.machine) }
}

/// Go's `os.Hostname`: the kernel's node name unless it may be truncated.
fn hostname(nodename: &str) -> String {
    if !nodename.is_empty() && nodename.len() < 64 {
        return nodename.to_owned();
    }
    fs::read_to_string("/proc/sys/kernel/hostname").map(|name| name.trim_end_matches('\n').to_owned()).unwrap_or_default()
}

/// gopsutil names ARM cores from the `CPU part` number.
fn arm_part_name(part: u64) -> &'static str {
    match part {
        0x810 => "ARM810",
        0x920 => "ARM920",
        0x922 => "ARM922",
        0x926 => "ARM926",
        0x940 => "ARM940",
        0x946 => "ARM946",
        0x966 => "ARM966",
        0xa20 => "ARM1020",
        0xa22 => "ARM1022",
        0xa26 => "ARM1026",
        0xb02 => "ARM11 MPCore",
        0xb36 => "ARM1136",
        0xb56 => "ARM1156",
        0xb76 => "ARM1176",
        0xc05 => "Cortex-A5",
        0xc07 => "Cortex-A7",
        0xc08 => "Cortex-A8",
        0xc09 => "Cortex-A9",
        0xc0d | 0xc0e => "Cortex-A17",
        0xc0f => "Cortex-A15",
        0xc14 => "Cortex-R4",
        0xc15 => "Cortex-R5",
        0xc17 => "Cortex-R7",
        0xc18 => "Cortex-R8",
        0xc20 => "Cortex-M0",
        0xc21 => "Cortex-M1",
        0xc23 => "Cortex-M3",
        0xc24 => "Cortex-M4",
        0xc27 => "Cortex-M7",
        0xc60 => "Cortex-M0+",
        0xd01 => "Cortex-A32",
        0xd02 => "Cortex-A34",
        0xd03 => "Cortex-A53",
        0xd04 => "Cortex-A35",
        0xd05 => "Cortex-A55",
        0xd06 => "Cortex-A65",
        0xd07 => "Cortex-A57",
        0xd08 => "Cortex-A72",
        0xd09 => "Cortex-A73",
        0xd0a => "Cortex-A75",
        0xd0b => "Cortex-A76",
        0xd0c => "Neoverse-N1",
        0xd0d => "Cortex-A77",
        0xd0e => "Cortex-A76AE",
        0xd13 => "Cortex-R52",
        0xd20 => "Cortex-M23",
        0xd21 => "Cortex-M33",
        0xd40 => "Neoverse-V1",
        0xd41 => "Cortex-A78",
        0xd42 => "Cortex-A78AE",
        0xd43 => "Cortex-A65AE",
        0xd44 => "Cortex-X1",
        0xd46 => "Cortex-A510",
        0xd47 => "Cortex-A710",
        0xd48 => "Cortex-X2",
        0xd49 => "Neoverse-N2",
        0xd4a => "Neoverse-E1",
        0xd4b => "Cortex-A78C",
        0xd4c => "Cortex-X1C",
        0xd4d => "Cortex-A715",
        0xd4e => "Cortex-X3",
        0xd4f => "Neoverse-V2",
        0xd81 => "Cortex-A720",
        0xd82 => "Cortex-X4",
        0xd84 => "Neoverse-V3",
        0xd85 => "Cortex-X925",
        0xd87 => "Cortex-A725",
        0xd8e => "Neoverse-N3",
        _ => "Undefined",
    }
}

/// Go's `strconv.ParseUint(value, 0, bits)` for the hex and decimal numbers cpuinfo holds.
fn parse_number(value: &str, bits: u32) -> Option<u64> {
    let number = match value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None if value.len() > 1 && value.starts_with('0') => u64::from_str_radix(&value[1..], 8).ok()?,
        None => value.parse().ok()?,
    };
    (number >> bits == 0).then_some(number)
}

/// The first CPU's model name as gopsutil's `cpu.Info` reads it, else the Go agent's fallback to the
/// `cpu model` and `system type` lines.
pub fn cpu_model(cpuinfo: &str) -> String {
    gopsutil_model(cpuinfo).filter(|model| !model.is_empty()).unwrap_or_else(|| fallback_model(cpuinfo))
}

fn gopsutil_model(cpuinfo: &str) -> Option<String> {
    let mut processor_name = String::new();
    let mut started = false;
    let mut model = String::new();
    let mut arm = false;
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Processor" => value.clone_into(&mut processor_name),
            "processor" | "cpu number" => {
                // gopsutil fails the whole read on a malformed number, even after a complete CPU.
                value.parse::<i64>().ok()?;
                if started {
                    return Some(model);
                }
                started = true;
                model.clone_from(&processor_name);
                arm = false;
            }
            "vendorId" | "vendor_id" => {
                arm = false;
                if value.contains("S390") {
                    "S390".clone_into(&mut processor_name);
                }
            }
            "CPU implementer" => {
                // Implementers gopsutil does not know leave the vendor as it was.
                const OTHER_VENDORS: [u64; 14] = [0x42, 0x43, 0x44, 0x46, 0x48, 0x49, 0x4d, 0x4e, 0x50, 0x51, 0x56, 0x61, 0x69, 0xc0];
                match parse_number(value, 8) {
                    Some(0x41) => arm = true,
                    Some(vendor) if OTHER_VENDORS.contains(&vendor) => arm = false,
                    _ => {}
                }
            }
            "model" | "CPU part" | "mimpid" => {
                if arm && let Some(part) = parse_number(value, 16) {
                    arm_part_name(part).clone_into(&mut model);
                }
            }
            "Model Name" | "model name" | "cpu" | "uarch" => value.clone_into(&mut model),
            _ => {}
        }
    }
    started.then_some(model)
}

fn fallback_model(cpuinfo: &str) -> String {
    let mut parts = Vec::new();
    for key in ["cpu model", "system type"] {
        let value = cpuinfo.lines().find_map(|line| {
            let rest = line.strip_prefix(key)?.trim();
            let value = rest.strip_prefix(':')?.trim();
            (rest.len() >= 2 && !value.is_empty()).then_some(value)
        });
        parts.extend(value);
    }
    parts.join(" / ")
}

/// gopsutil's logical CPU count: numbered `processor` lines, else the `cpuN` lines of `/proc/stat`.
fn count_threads(cpuinfo: &str, stat: &str) -> u64 {
    let numbered = cpuinfo
        .lines()
        .map(str::to_lowercase)
        .filter(|line| line.starts_with("processor"))
        .filter(|line| line[line.find(':').map_or(0, |at| at + 1)..].trim().parse::<i32>().is_ok())
        .count();
    if numbered > 0 {
        return numbered as u64;
    }
    stat.lines().filter(|line| line.len() >= 4 && line.starts_with("cpu") && line.as_bytes()[3].is_ascii_digit()).count() as u64
}

/// gopsutil's physical core count: distinct sibling lists in sysfs topology.
fn count_cores() -> u64 {
    let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") else { return 0 };
    let cpus: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_str().and_then(|name| name.strip_prefix("cpu")).is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit())))
        .map(|entry| entry.path())
        .collect();
    let mut lists: Vec<String> = Vec::new();
    for file in ["topology/core_cpus_list", "topology/thread_siblings_list"] {
        for cpu in &cpus {
            if let Ok(text) = fs::read_to_string(cpu.join(file)) {
                let lines: Vec<&str> = text.lines().collect();
                if let [line] = lines[..]
                    && !lists.iter().any(|known| known == line)
                {
                    lists.push(line.to_owned());
                }
            }
        }
        if !lists.is_empty() {
            return lists.len() as u64;
        }
    }
    0
}

/// `PRETTY_NAME` from os-release, else its `ID`.
fn os_name(os_release: &str) -> String {
    let field = |key: &str| os_release.lines().find_map(|line| line.strip_prefix(key)).map(|value| value.trim_matches('"').to_owned());
    field("PRETTY_NAME=").or_else(|| field("ID=")).unwrap_or_default()
}

pub fn details(memory_total: u64) -> Details {
    let uts = uname();
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let threads = count_threads(&cpuinfo, &fs::read_to_string("/proc/stat").unwrap_or_default());
    let mut cores = count_cores();
    // A container's logical CPUs can be fewer than the host's cores.
    if threads > 0 && threads < cores {
        cores = threads;
    }
    Details {
        hostname: hostname(&uts.nodename),
        kernel: uts.release,
        cores,
        threads,
        cpu_model: cpu_model(&cpuinfo),
        os_name: os_name(&fs::read_to_string("/etc/os-release").unwrap_or_default()),
        arch: uts.machine,
        memory_total,
    }
}

/// Seconds since boot.
pub fn uptime() -> u64 {
    let from_proc = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
    if let Some(seconds) = from_proc {
        return seconds as u64;
    }
    // SAFETY: `sysinfo` fills the zeroed struct.
    let mut info: libc::sysinfo = unsafe { std::mem::zeroed() };
    if unsafe { libc::sysinfo(&mut info) } != 0 {
        return 0;
    }
    u64::try_from(info.uptime).unwrap_or(0)
}

pub fn load_avg() -> Option<[f64; 3]> {
    let text = fs::read_to_string("/proc/loadavg").ok()?;
    let mut fields = text.split_whitespace().map(|field| field.parse::<f64>().ok().map(two_decimals));
    Some([fields.next()??, fields.next()??, fields.next()??])
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".write-test");
    let created = fs::File::create(&probe).is_ok();
    if created {
        let _ = fs::remove_file(&probe);
    }
    created
}

/// The first usable of `DATA_DIR`, `/var/lib/beszel-agent` and `~/.config/beszel`: an existing
/// writable directory, else one that can be created.
pub fn data_dir(configured: &str, home: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if !configured.is_empty() {
        candidates.push(PathBuf::from(configured));
    }
    candidates.push(PathBuf::from("/var/lib/beszel-agent"));
    if !home.is_empty() {
        candidates.push(Path::new(home).join(".config/beszel"));
    }
    if let Some(dir) = candidates.iter().find(|dir| dir.is_dir() && writable(dir)) {
        return Some(dir.clone());
    }
    candidates
        .into_iter()
        .find(|dir| !dir.exists() && fs::DirBuilder::new().recursive(true).mode(0o755).create(dir).is_ok() && writable(dir))
}

const FINGERPRINT_FILE: &str = "fingerprint";
/// A placeholder UUID some firmware ships for every board.
const KNOWN_BAD_UUID: &str = "03000200-0400-0500-0006-000700080009";

/// gopsutil's `HostID`.
fn host_id() -> String {
    let first_line = |path: &str| fs::read_to_string(path).ok().and_then(|text| text.lines().next().map(str::to_owned)).filter(|line| !line.is_empty());
    if let Some(uuid) = first_line("/sys/class/dmi/id/product_uuid") {
        return uuid.to_lowercase();
    }
    if let Some(id) = first_line("/etc/machine-id").filter(|id| id.len() == 32 && id.is_ascii()) {
        return format!("{}-{}-{}-{}-{}", &id[0..8], &id[8..12], &id[12..16], &id[16..20], &id[20..32]);
    }
    first_line("/proc/sys/kernel/random/boot_id").map(|id| id.to_lowercase()).unwrap_or_default()
}

fn generate_fingerprint(host_id: &str, hostname: &str, cpu_model: &str) -> String {
    let source = if host_id.is_empty() || host_id == KNOWN_BAD_UUID { format!("{hostname}{cpu_model}") } else { host_id.to_owned() };
    let digest = ring::digest::digest(&ring::digest::SHA256, source.as_bytes());
    digest.as_ref()[..24].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The identity the hub knows this machine by: the saved one, else a new one saved for next time.
pub fn fingerprint(data_dir: Option<&Path>, hostname: &str, cpu_model: &str) -> String {
    if let Some(dir) = data_dir
        && let Ok(saved) = fs::read(dir.join(FINGERPRINT_FILE))
    {
        let saved = String::from_utf8_lossy(&saved).trim().to_owned();
        if !saved.is_empty() {
            return saved;
        }
    }
    let fingerprint = generate_fingerprint(&host_id(), hostname, cpu_model);
    if let Some(dir) = data_dir {
        let file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).open(dir.join(FINGERPRINT_FILE));
        let _ = file.and_then(|mut file| file.write_all(fingerprint.as_bytes()));
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_saved_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FINGERPRINT_FILE), "0123abcd\n").unwrap();
        assert_eq!(fingerprint(Some(dir.path()), "tv", "Cortex-A55"), "0123abcd");

        fs::write(dir.path().join(FINGERPRINT_FILE), " \n").unwrap();
        let made = fingerprint(Some(dir.path()), "tv", "Cortex-A55");
        assert_eq!(made.len(), 48);
        assert_eq!(fs::read_to_string(dir.path().join(FINGERPRINT_FILE)).unwrap(), made);
        assert_eq!(fingerprint(Some(dir.path()), "other", "other"), made);
    }

    #[test]
    fn fingerprints_like_the_go_agent() {
        // sha256("tvCortex-A55"), first 24 bytes.
        let expected = ring::digest::digest(&ring::digest::SHA256, b"tvCortex-A55");
        let hex: String = expected.as_ref()[..24].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(generate_fingerprint(KNOWN_BAD_UUID, "tv", "Cortex-A55"), hex);
        assert_ne!(generate_fingerprint("abc", "tv", "Cortex-A55"), hex);
    }

    #[test]
    fn names_the_cpu_like_gopsutil() {
        let arm = "processor\t: 0\nBogoMIPS\t: 48.00\nCPU implementer\t: 0x41\nCPU part\t: 0xd05\n\n\
                   processor\t: 1\nCPU implementer\t: 0x41\nCPU part\t: 0xd03\n";
        assert_eq!(cpu_model(arm), "Cortex-A55");
        assert_eq!(count_threads(arm, ""), 2);
        assert_eq!(cpu_model("processor : 0\nmodel name : Intel(R) N100\n"), "Intel(R) N100");
        assert_eq!(cpu_model("system type : MT7621\nprocessor : 0\ncpu model : MIPS 1004Kc\n"), "MIPS 1004Kc / MT7621");
        assert_eq!(count_threads("", "cpu  1 2\ncpu0 1 2\ncpu1 1 2\nintr 0\n"), 2);
        assert_eq!(os_name("NAME=webOS\nID=webos\n"), "webos");
        assert_eq!(os_name("PRETTY_NAME=\"Debian GNU/Linux 12\"\n"), "Debian GNU/Linux 12");
    }
}
