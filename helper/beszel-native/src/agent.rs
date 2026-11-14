//! The report for a data request, collected as the Go agent collects it, with its per-interval cache.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::cpu::{self, Cpu};
use crate::disk::{self, Disks};
use crate::gpu::Gpus;
use crate::host;
use crate::mem::{self, Memory};
use crate::net::Net;
use crate::proto::DataOptions;
use crate::sensors::Sensors;
use crate::slog;
use crate::system::{self, Details, Info, Stats, gigabytes, two_decimals};
use crate::wifi;

/// The hub's regular request interval; the only one that refreshes Wi-Fi and carries details.
pub const DEFAULT_INTERVAL: u16 = 60_000;

/// An environment lookup: `BESZEL_AGENT_<KEY>` before `<KEY>`, as the Go agent reads its settings.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

pub struct Agent {
    pub data_dir: Option<PathBuf>,
    pub details: Details,
    details_dirty: bool,
    htop: bool,
    cpu: Cpu,
    disks: Disks,
    net: Net,
    sensors: Sensors,
    primary_sensor: String,
    gpus: Option<Gpus>,
    skip_wifi: bool,
    wifi: Vec<(String, f64)>,
    cache: HashMap<u16, (Instant, Vec<u8>)>,
}

fn read_stat() -> Result<cpu::Stat, String> {
    let text = fs::read_to_string("/proc/stat").map_err(|err| err.to_string())?;
    cpu::parse_stat(&text).ok_or_else(|| "malformed /proc/stat".to_owned())
}

fn read_memory(htop: bool) -> Option<Memory> {
    mem::parse(&fs::read_to_string("/proc/meminfo").ok()?, htop)
}

impl Agent {
    pub fn new(env: Env) -> Agent {
        let var = |key: &str| env(key).unwrap_or_default();
        let data_dir = host::data_dir(&var("DATA_DIR"), &std::env::var("HOME").unwrap_or_default());
        match &data_dir {
            Some(dir) => slog::info("Data directory", &[("path", &dir.display())]),
            None => slog::warn("Data directory not found", &[]),
        }
        let htop = var("MEM_CALC") == "htop";
        let skip_gpu = var("SKIP_GPU") == "true";
        let primary_sensor = var("PRIMARY_SENSOR");
        let sensors = Sensors::new(
            &var("SYS_SENSORS"),
            &std::env::var("HOST_SYS").unwrap_or_default(),
            env("SENSORS").as_deref(),
            &primary_sensor,
            skip_gpu,
        );
        if let Some(level) = env("LOG_LEVEL") {
            slog::set_level(&level);
        }
        let details = host::details(read_memory(htop).map_or(0, |memory| memory.total));
        let disks = Disks::discover(&disk::Config { filesystem: &var("FILESYSTEM"), extra: env("EXTRA_FILESYSTEMS").as_deref() });
        let net = Net::new(env("NICS").as_deref());
        if skip_gpu {
            slog::info("SKIP_GPU enabled, skipping GPU monitoring (collectors, temperatures, and fans)", &[]);
        }
        let gpus = env("GPU_SAMPLE_FILE").filter(|path| !path.is_empty() && !skip_gpu).map(|path| Gpus::new(path.into()));
        let skip_wifi = var("SKIP_WIFI") == "true";
        if skip_wifi {
            slog::info("SKIP_WIFI enabled, skipping Wi-Fi signal monitoring", &[]);
        }
        Agent {
            data_dir,
            details,
            details_dirty: false,
            htop,
            cpu: Cpu::new(read_stat().ok()),
            disks,
            net,
            sensors,
            primary_sensor,
            gpus,
            skip_wifi,
            wifi: Vec::new(),
            cache: HashMap::new(),
        }
    }

    /// The encoded `CombinedData` for a request. Requests for the same interval within half of it
    /// share one collection, so overlapping hub clients do not shorten each other's spans.
    pub fn gather(&mut self, options: DataOptions) -> Vec<u8> {
        let interval = options.cache_time_ms;
        if let Some((at, data)) = self.cache.get(&interval)
            && at.elapsed() < Duration::from_millis(u64::from(interval / 2))
        {
            slog::debug("Cached data", &[("cacheTimeMs", &interval)]);
            return data.clone();
        }
        let (stats, info) = self.collect(interval);
        let data = system::encode(&stats, &info, None);
        self.cache.insert(interval, (Instant::now(), data.clone()));
        if interval != DEFAULT_INTERVAL || !(options.include_details || self.details_dirty) {
            return data;
        }
        self.details_dirty = false;
        system::encode(&stats, &info, Some(&self.details))
    }

    fn collect(&mut self, interval: u16) -> (Stats, Info) {
        let mut stats = Stats::default();
        let mut info = Info::default();
        match read_stat() {
            Ok(stat) => {
                let (usage, cores) = self.cpu.sample(interval, stat);
                stats.cpu = two_decimals(usage.total);
                stats.cpu_breakdown = Some(usage.breakdown.map(two_decimals));
                stats.cpu_cores = cores;
            }
            Err(err) => slog::error("Error getting cpu metrics", &[("err", &err)]),
        }
        match host::load_avg() {
            Some(load) => stats.load_avg = load,
            None => slog::error("Error getting load average", &[("err", &"unreadable /proc/loadavg")]),
        }
        self.update_memory(&mut stats);
        self.disks.update_usage(&mut stats);
        self.disks.update_io(interval, &mut stats);
        self.net.update(interval, &mut stats);
        if let Some(reading) = self.sensors.reading() {
            stats.temperatures = reading.temperatures;
            info.dashboard_temp = reading.dashboard;
        }
        if let Some(gpus) = &mut self.gpus {
            let current = gpus.current(interval);
            if !current.is_empty() {
                let mut highest = 0.0_f64;
                for (_, gpu) in &current {
                    if gpu.temperature > 0.0 {
                        stats.temperatures.retain(|(name, _)| *name != gpu.name);
                        stats.temperatures.push((gpu.name.clone(), gpu.temperature));
                        if self.primary_sensor == gpu.name {
                            info.dashboard_temp = gpu.temperature;
                        }
                        highest = highest.max(gpu.temperature);
                    }
                    info.gpu_pct = info.gpu_pct.max(gpu.usage);
                }
                if info.dashboard_temp == 0.0 {
                    info.dashboard_temp = highest;
                }
                stats.gpus = current;
            }
        }
        // The Go agent reads Wi-Fi only on the regular interval; live requests reuse that reading.
        if interval == DEFAULT_INTERVAL && !self.skip_wifi {
            self.wifi = wifi::collect();
        }
        stats.wifi = wifi::signals(&self.wifi);
        info.wifi.clone_from(&self.wifi);

        info.cpu = stats.cpu;
        info.load_avg = stats.load_avg;
        info.mem_pct = stats.mem_pct;
        info.disk_pct = stats.disk_pct;
        info.uptime = host::uptime();
        info.bandwidth_bytes = stats.bandwidth[0] + stats.bandwidth[1];
        info.threads = self.details.threads;
        self.disks.root_name().clone_into(&mut info.root_disk_name);
        for (name, fs) in self.disks.extra() {
            stats.extra_fs.push((name.to_owned(), fs.clone()));
            info.extra_fs_pct.push((name.to_owned(), two_decimals(fs.used / fs.total * 100.0)));
        }
        (stats, info)
    }

    fn update_memory(&mut self, stats: &mut Stats) {
        let Some(memory) = read_memory(self.htop) else { return };
        stats.swap_bytes = [memory.swap_total, memory.swap_used];
        stats.swap = gigabytes(memory.swap_total);
        stats.swap_used = gigabytes(memory.swap_used);
        stats.mem_bytes = [memory.total, memory.used, memory.buff_cache, 0];
        stats.mem = gigabytes(memory.total);
        stats.mem_used = gigabytes(memory.used);
        stats.mem_buff_cache = gigabytes(memory.buff_cache);
        if memory.total > 0 {
            stats.mem_pct = two_decimals(memory.used as f64 / memory.total as f64 * 100.0);
        }
        if self.details.memory_total != memory.total {
            self.details.memory_total = memory.total;
            self.details_dirty = true;
        }
    }
}
