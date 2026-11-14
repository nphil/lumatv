//! Temperatures from a sysfs hwmon tree (`SYS_SENSORS`, else `HOST_SYS`, else `/sys`), read as gopsutil
//! reads them and filtered as `agent/sensors.go` filters them.

use std::path::{Path, PathBuf};

use crate::filter::NameFilter;
use crate::slog;
use crate::system::two_decimals;

/// Hwmon chips that belong to GPUs; `SKIP_GPU` leaves them unread.
const GPU_CHIPS: [&str; 6] = ["xe", "i915", "amdgpu", "radeon", "nvidia", "nouveau"];

pub struct Sensors {
    root: PathBuf,
    /// `None` when `SENSORS` is set but empty: no temperatures at all.
    filter: Option<NameFilter>,
    primary: String,
    skip_gpu: bool,
}

/// What one reading reports: each sensor and the dashboard's temperature.
#[derive(Debug, Default, PartialEq)]
pub struct Reading {
    pub temperatures: Vec<(String, f64)>,
    pub dashboard: f64,
}

fn sorted_entries(dir: &Path, prefix: &str, suffix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.len() >= prefix.len() + suffix.len() && name.starts_with(prefix) && name.ends_with(suffix))
        .collect();
    names.sort();
    names
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|text| text.trim().to_owned())
}

fn is_gpu_chip(name: &str) -> bool {
    GPU_CHIPS.contains(&name.trim().to_lowercase().as_str())
}

fn is_gpu_sensor(key: &str) -> bool {
    let key = key.trim().to_lowercase();
    GPU_CHIPS.iter().any(|chip| key == *chip || key.starts_with(&format!("{chip}_")))
}

/// 0.52 is a sensor that reports in tens of degrees or in degrees divided by a thousand.
fn scale(temperature: f64) -> f64 {
    if temperature > 1.0 {
        return temperature;
    }
    let (by_100, by_1000) = (temperature * 100.0, temperature * 1000.0);
    if (15.0..=95.0).contains(&by_100) {
        by_100
    } else if (15.0..=95.0).contains(&by_1000) {
        by_1000
    } else {
        by_100
    }
}

impl Sensors {
    pub fn new(sys_sensors: &str, host_sys: &str, sensors: Option<&str>, primary: &str, skip_gpu: bool) -> Sensors {
        let root = if !sys_sensors.is_empty() {
            slog::info("SYS_SENSORS", &[("path", &sys_sensors)]);
            sys_sensors
        } else if !host_sys.is_empty() {
            host_sys
        } else {
            "/sys"
        };
        let filter = match sensors {
            Some("") => None,
            value => Some(NameFilter::parse(value.unwrap_or_default())),
        };
        Sensors { root: PathBuf::from(root), filter, primary: primary.to_owned(), skip_gpu }
    }

    /// gopsutil's `TemperaturesWithContext`: `temp*_input` of each hwmon chip, in directory order.
    fn read(&self) -> Vec<(String, f64)> {
        let hwmon = self.root.join("class/hwmon");
        let chips: Vec<PathBuf> = sorted_entries(&hwmon, "hwmon", "")
            .into_iter()
            .map(|chip| hwmon.join(chip))
            .filter(|chip| {
                !self.skip_gpu || read_trimmed(&chip.join("name")).or_else(|| read_trimmed(&chip.join("device/name"))).is_some_and(|name| !is_gpu_chip(&name))
            })
            .collect();
        let inputs = |subdir: &str| -> Vec<PathBuf> {
            chips
                .iter()
                .flat_map(|chip| {
                    let dir = chip.join(subdir);
                    sorted_entries(&dir, "temp", "_input").into_iter().map(move |file| dir.join(file))
                })
                .collect()
        };
        let mut files = inputs("");
        if files.is_empty() {
            files = inputs("device");
        }
        let mut out = Vec::with_capacity(files.len());
        for file in files {
            let Some(dir) = file.parent() else { continue };
            let file_name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let base = file_name.split('_').next().unwrap_or_default();
            let Some(mut name) = read_trimmed(&dir.join("name")) else { continue };
            let label = read_trimmed(&dir.join(format!("{base}_label"))).unwrap_or_default().to_lowercase().replace(' ', "_");
            if !label.is_empty() {
                name = format!("{name}_{label}");
            }
            let Some(value) = read_trimmed(&file).and_then(|value| value.parse::<f64>().ok()) else { continue };
            out.push((name, value / 1000.0));
        }
        out
    }

    /// `None` when `SENSORS` turns temperatures off.
    pub fn reading(&self) -> Option<Reading> {
        let Some(filter) = &self.filter else {
            slog::debug("Skipping temperature collection", &[]);
            return None;
        };
        Some(self.select(filter, self.read()))
    }

    fn select(&self, filter: &NameFilter, temps: Vec<(String, f64)>) -> Reading {
        let mut reading = Reading::default();
        let mut claimed: Vec<String> = Vec::with_capacity(temps.len());
        for (i, (mut name, mut temperature)) in temps.into_iter().enumerate() {
            if claimed.contains(&name) {
                name = format!("{name}_{i}");
            }
            claimed.push(name.clone());
            if temperature != 0.0 && temperature < 1.0 {
                temperature = scale(temperature);
            }
            if temperature <= 0.0 || temperature >= 200.0 || !filter.allows(&name, true) || (self.skip_gpu && is_gpu_sensor(&name)) {
                continue;
            }
            if self.primary.is_empty() {
                reading.dashboard = reading.dashboard.max(temperature);
            } else if self.primary == name {
                reading.dashboard = temperature;
            }
            reading.temperatures.retain(|(n, _)| *n != name);
            reading.temperatures.push((name, two_decimals(temperature)));
        }
        reading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(root: &Path, dir: &str, name: &str, temps: &[(u32, &str, &str)]) {
        let dir = root.join("class/hwmon").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("name"), format!("{name}\n")).unwrap();
        for (index, label, value) in temps {
            std::fs::write(dir.join(format!("temp{index}_input")), value).unwrap();
            if !label.is_empty() {
                std::fs::write(dir.join(format!("temp{index}_label")), label).unwrap();
            }
        }
    }

    #[test]
    fn reads_hwmon_like_gopsutil() {
        let dir = tempfile::tempdir().unwrap();
        chip(dir.path(), "hwmon0", "soc", &[(1, "", "52000\n")]);
        chip(dir.path(), "hwmon1", "coretemp", &[(1, "Core 0", "61500"), (2, "Core 1", "450"), (3, "", "250000")]);
        chip(dir.path(), "hwmon2", "amdgpu", &[(1, "edge", "70000")]);

        let all = Sensors::new(&dir.path().to_string_lossy(), "", None, "", false);
        let reading = all.reading().unwrap();
        assert_eq!(
            reading.temperatures,
            [
                ("soc".to_owned(), 52.0),
                ("coretemp_core_0".to_owned(), 61.5),
                ("coretemp_core_1".to_owned(), 45.0),
                ("amdgpu_edge".to_owned(), 70.0),
            ]
        );
        assert_eq!(reading.dashboard, 70.0);

        let chosen = Sensors::new(&dir.path().to_string_lossy(), "", Some("soc,coretemp*"), "soc", true);
        let reading = chosen.reading().unwrap();
        assert_eq!(reading.temperatures.len(), 3);
        assert_eq!(reading.dashboard, 52.0);

        assert_eq!(Sensors::new(&dir.path().to_string_lossy(), "", Some(""), "", false).reading(), None);
    }
}
