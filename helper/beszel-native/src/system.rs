//! The report the hub stores (`internal/entities/system/system.go` upstream). Fields the TV never has
//! (containers, batteries, ZFS, systemd) are left out; empty optional fields are omitted as the Go
//! structs' `omitempty` tags omit them.

use crate::cbor::{Enc, Map};

/// The upstream agent release whose protocol this agent speaks. The hub checks it at connect.
pub const UPSTREAM: &str = "0.21.1";

pub fn two_decimals(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub fn gigabytes(bytes: u64) -> f64 {
    two_decimals(bytes as f64 / 1_073_741_824.0)
}

pub fn megabytes(bytes: f64) -> f64 {
    two_decimals(bytes / 1_048_576.0)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats {
    pub cpu: f64,
    pub mem: f64,
    pub mem_used: f64,
    pub mem_pct: f64,
    pub mem_buff_cache: f64,
    pub swap: f64,
    pub swap_used: f64,
    pub disk_total: f64,
    pub disk_used: f64,
    pub disk_pct: f64,
    pub temperatures: Vec<(String, f64)>,
    pub extra_fs: Vec<(String, FsStats)>,
    pub gpus: Vec<(String, Gpu)>,
    /// Bytes per second sent and received.
    pub bandwidth: [u64; 2],
    pub load_avg: [f64; 3],
    /// Per interface: bytes per second up and down, then the totals sent and received.
    pub interfaces: Vec<(String, [u64; 4])>,
    pub disk_io: [u64; 2],
    /// user, system, iowait, steal, idle, irq, softirq, nice.
    pub cpu_breakdown: Option<[f64; 8]>,
    pub cpu_cores: Vec<u8>,
    pub disk_io_stats: [f64; 6],
    pub disk_io_total: [u64; 2],
    pub wifi: Vec<(String, i8)>,
    /// Total, used, buffers and cache, ZFS ARC.
    pub mem_bytes: [u64; 4],
    pub swap_bytes: [u64; 2],
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FsStats {
    pub total: f64,
    pub used: f64,
    pub total_read: u64,
    pub total_write: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    /// Read time %, write time %, utilisation %, read await ms, write await ms, weighted I/O %.
    pub io_stats: [f64; 6],
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// Reported among the temperatures, not inside the GPU entry.
    pub temperature: f64,
    pub mem_used: f64,
    pub mem_total: f64,
    pub usage: f64,
    pub power: f64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Info {
    pub threads: u64,
    pub uptime: u64,
    pub cpu: f64,
    pub mem_pct: f64,
    pub disk_pct: f64,
    pub gpu_pct: f64,
    pub dashboard_temp: f64,
    pub bandwidth_bytes: u64,
    pub load_avg: [f64; 3],
    pub extra_fs_pct: Vec<(String, f64)>,
    pub root_disk_name: String,
    /// Signal in dBm per connected interface.
    pub wifi: Vec<(String, f64)>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Details {
    pub hostname: String,
    pub kernel: String,
    pub cores: u64,
    pub threads: u64,
    pub cpu_model: String,
    pub os_name: String,
    pub arch: String,
    pub memory_total: u64,
}

/// Info.ConnectionType: the agent dialled the hub's WebSocket.
const CONNECTION_WEBSOCKET: u64 = 2;

fn float(e: &mut Enc, m: &mut Map, key: u64, value: f64) {
    e.key(m, key);
    e.f64(value);
}

fn nonzero_float(e: &mut Enc, m: &mut Map, key: u64, value: f64) {
    if value != 0.0 {
        float(e, m, key, value);
    }
}

fn floats(e: &mut Enc, values: &[f64]) {
    e.array(values.len());
    for &value in values {
        e.f64(value);
    }
}

fn uints(e: &mut Enc, values: &[u64]) {
    e.array(values.len());
    for &value in values {
        e.uint(value);
    }
}

impl Stats {
    fn encode(&self, e: &mut Enc) {
        let mut m = e.begin_map();
        float(e, &mut m, 0, self.cpu);
        float(e, &mut m, 2, self.mem);
        float(e, &mut m, 3, self.mem_used);
        float(e, &mut m, 4, self.mem_pct);
        float(e, &mut m, 5, self.mem_buff_cache);
        nonzero_float(e, &mut m, 7, self.swap);
        nonzero_float(e, &mut m, 8, self.swap_used);
        float(e, &mut m, 9, self.disk_total);
        float(e, &mut m, 10, self.disk_used);
        float(e, &mut m, 11, self.disk_pct);
        if !self.temperatures.is_empty() {
            e.key(&mut m, 20);
            e.map(self.temperatures.len());
            for (name, value) in &self.temperatures {
                e.text(name);
                e.f64(*value);
            }
        }
        if !self.extra_fs.is_empty() {
            e.key(&mut m, 21);
            e.map(self.extra_fs.len());
            for (name, fs) in &self.extra_fs {
                e.text(name);
                fs.encode(e);
            }
        }
        if !self.gpus.is_empty() {
            e.key(&mut m, 22);
            e.map(self.gpus.len());
            for (id, gpu) in &self.gpus {
                e.text(id);
                gpu.encode(e);
            }
        }
        if self.bandwidth != [0; 2] {
            e.key(&mut m, 26);
            uints(e, &self.bandwidth);
        }
        e.key(&mut m, 28);
        floats(e, &self.load_avg);
        if !self.interfaces.is_empty() {
            e.key(&mut m, 31);
            e.map(self.interfaces.len());
            for (name, values) in &self.interfaces {
                e.text(name);
                uints(e, values);
            }
        }
        if self.disk_io != [0; 2] {
            e.key(&mut m, 32);
            uints(e, &self.disk_io);
        }
        if let Some(breakdown) = &self.cpu_breakdown {
            e.key(&mut m, 33);
            floats(e, breakdown);
        }
        if !self.cpu_cores.is_empty() {
            e.key(&mut m, 34);
            e.bytes(&self.cpu_cores);
        }
        if self.disk_io_stats != [0.0; 6] {
            e.key(&mut m, 35);
            floats(e, &self.disk_io_stats);
        }
        if self.disk_io_total != [0; 2] {
            e.key(&mut m, 38);
            uints(e, &self.disk_io_total);
        }
        if !self.wifi.is_empty() {
            e.key(&mut m, 40);
            e.map(self.wifi.len());
            for (name, signal) in &self.wifi {
                e.text(name);
                e.int((*signal).into());
            }
        }
        if self.mem_bytes != [0; 4] {
            e.key(&mut m, 41);
            uints(e, &self.mem_bytes);
        }
        if self.swap_bytes != [0; 2] {
            e.key(&mut m, 42);
            uints(e, &self.swap_bytes);
        }
        e.end_map(m);
    }
}

impl FsStats {
    fn encode(&self, e: &mut Enc) {
        let mut m = e.begin_map();
        float(e, &mut m, 0, self.total);
        float(e, &mut m, 1, self.used);
        for (key, value) in [(9, self.total_read), (10, self.total_write), (6, self.read_bytes), (7, self.write_bytes)] {
            if value != 0 {
                e.key(&mut m, key);
                e.uint(value);
            }
        }
        if self.io_stats != [0.0; 6] {
            e.key(&mut m, 8);
            floats(e, &self.io_stats);
        }
        e.end_map(m);
    }
}

impl Gpu {
    fn encode(&self, e: &mut Enc) {
        let mut m = e.begin_map();
        e.key(&mut m, 0);
        e.text(&self.name);
        nonzero_float(e, &mut m, 1, self.mem_used);
        nonzero_float(e, &mut m, 2, self.mem_total);
        nonzero_float(e, &mut m, 3, self.usage);
        nonzero_float(e, &mut m, 4, self.power);
        e.end_map(m);
    }
}

impl Info {
    fn encode(&self, e: &mut Enc) {
        let mut m = e.begin_map();
        if self.threads != 0 {
            e.key(&mut m, 3);
            e.uint(self.threads);
        }
        e.key(&mut m, 5);
        e.uint(self.uptime);
        float(e, &mut m, 6, self.cpu);
        float(e, &mut m, 7, self.mem_pct);
        float(e, &mut m, 8, self.disk_pct);
        // The deprecated bandwidth in MB/s has no omitempty upstream.
        float(e, &mut m, 9, 0.0);
        e.key(&mut m, 10);
        e.text(UPSTREAM);
        nonzero_float(e, &mut m, 12, self.gpu_pct);
        nonzero_float(e, &mut m, 13, self.dashboard_temp);
        e.key(&mut m, 18);
        e.uint(self.bandwidth_bytes);
        e.key(&mut m, 19);
        floats(e, &self.load_avg);
        e.key(&mut m, 20);
        e.uint(CONNECTION_WEBSOCKET);
        if !self.extra_fs_pct.is_empty() {
            e.key(&mut m, 21);
            e.map(self.extra_fs_pct.len());
            for (name, pct) in &self.extra_fs_pct {
                e.text(name);
                e.f64(*pct);
            }
        }
        if !self.root_disk_name.is_empty() {
            e.key(&mut m, 24);
            e.text(&self.root_disk_name);
        }
        if !self.wifi.is_empty() {
            e.key(&mut m, 26);
            e.map(self.wifi.len());
            for (name, signal) in &self.wifi {
                e.text(name);
                e.map(1);
                e.uint(1);
                e.f64(*signal);
            }
        }
        e.end_map(m);
    }
}

impl Details {
    fn encode(&self, e: &mut Enc) {
        let mut m = e.begin_map();
        e.key(&mut m, 0);
        e.text(&self.hostname);
        if !self.kernel.is_empty() {
            e.key(&mut m, 1);
            e.text(&self.kernel);
        }
        e.key(&mut m, 2);
        e.uint(self.cores);
        e.key(&mut m, 3);
        e.uint(self.threads);
        e.key(&mut m, 4);
        e.text(&self.cpu_model);
        // Os: Linux.
        e.key(&mut m, 5);
        e.uint(0);
        e.key(&mut m, 6);
        e.text(&self.os_name);
        e.key(&mut m, 7);
        e.text(&self.arch);
        e.key(&mut m, 9);
        e.uint(self.memory_total);
        e.end_map(m);
    }
}

/// `CombinedData`: the answer to a data request.
pub fn encode(stats: &Stats, info: &Info, details: Option<&Details>) -> Vec<u8> {
    let mut e = Enc::new();
    let mut m = e.begin_map();
    e.key(&mut m, 0);
    stats.encode(&mut e);
    e.key(&mut m, 1);
    info.encode(&mut e);
    // No containers and no network monitors; the Go agent sends both as null.
    e.key(&mut m, 2);
    e.null();
    if let Some(details) = details {
        e.key(&mut m, 4);
        details.encode(&mut e);
    }
    e.key(&mut m, 6);
    e.null();
    e.end_map(m);
    e.buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::Dec;

    /// The keys of a CBOR map, in order, with each value's raw bytes.
    fn fields(item: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let mut out = Vec::new();
        Dec::new(item)
            .map(|d, key| {
                out.push((key.unwrap(), d.item()?.to_vec()));
                Ok(())
            })
            .unwrap();
        out
    }

    fn keys(item: &[u8]) -> Vec<u64> {
        fields(item).into_iter().map(|(key, _)| key).collect()
    }

    #[test]
    fn combined_data_carries_the_go_agents_keys() {
        let stats = Stats {
            cpu: 1.5,
            swap: 0.5,
            temperatures: vec![("soc".into(), 44.0)],
            extra_fs: vec![("Data".into(), FsStats { total: 1.0, used: 0.5, ..FsStats::default() })],
            gpus: vec![("n0".into(), Gpu { name: "Mali".into(), usage: 12.0, ..Gpu::default() })],
            bandwidth: [10, 20],
            cpu_breakdown: Some([0.0; 8]),
            cpu_cores: vec![3, 4],
            mem_bytes: [1, 2, 3, 0],
            wifi: vec![("wlan0".into(), -60)],
            ..Stats::default()
        };
        let info = Info { threads: 4, extra_fs_pct: vec![("Data".into(), 50.0)], root_disk_name: "Apps".into(), ..Info::default() };
        let details = Details { hostname: "lgtv".into(), ..Details::default() };

        let top = fields(&encode(&stats, &info, Some(&details)));
        assert_eq!(top.iter().map(|(key, _)| *key).collect::<Vec<_>>(), [0, 1, 2, 4, 6]);
        assert_eq!((top[2].1.as_slice(), top[4].1.as_slice()), (&[0xf6][..], &[0xf6][..]));
        assert_eq!(keys(&top[0].1), [0, 2, 3, 4, 5, 7, 9, 10, 11, 20, 21, 22, 26, 28, 33, 34, 40, 41]);
        assert_eq!(keys(&top[1].1), [3, 5, 6, 7, 8, 9, 10, 18, 19, 20, 21, 24]);
        assert_eq!(keys(&top[3].1), [0, 2, 3, 4, 5, 6, 7, 9]);

        let info_fields = fields(&top[1].1);
        let version = &info_fields.iter().find(|(key, _)| *key == 10).unwrap().1;
        assert_eq!(Dec::new(version).text().unwrap(), UPSTREAM);

        // Without details the data request's answer has no key 4.
        assert_eq!(keys(&encode(&stats, &info, None)), [0, 1, 2, 6]);
    }

    #[test]
    fn rounds_as_the_go_agent_does() {
        assert_eq!(two_decimals(12.345_678), 12.35);
        assert_eq!(gigabytes(2 << 30), 2.0);
        assert_eq!(megabytes(1_572_864.0), 1.5);
    }
}
