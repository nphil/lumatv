//! GPU use from a file holding one `nvtop -lP` sample (`GPU_SAMPLE_FILE`), averaged per request
//! interval as the Go agent's nvtop collector averages its stream (`agent/gpu_nvtop.go`, `agent/gpu.go`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::system::{Gpu, megabytes, two_decimals};

/// One device of an nvtop sample; `None` is a JSON null or an absent field.
#[derive(Debug, Default, PartialEq)]
pub struct Sample {
    pub name: String,
    pub temp: Option<String>,
    pub power: Option<String>,
    pub util: Option<String>,
    pub mem_total: Option<String>,
    pub mem_used: Option<String>,
}

/// Just enough JSON for nvtop's output: an array of objects whose known fields are strings or null.
/// Anything else in a known field makes the sample invalid, as Go's decoder rejects it.
struct Json<'a> {
    text: &'a [u8],
    at: usize,
}

impl Json<'_> {
    fn peek(&mut self) -> Option<u8> {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
        self.text.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn word(&mut self, word: &[u8]) -> Option<()> {
        self.peek();
        self.text[self.at..].starts_with(word).then(|| self.at += word.len())
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = std::str::from_utf8(self.text.get(self.at..self.at + 4)?).ok()?;
        self.at += 4;
        u32::from_str_radix(digits, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') {
            return None;
        }
        let mut out = Vec::new();
        loop {
            let byte = *self.text.get(self.at)?;
            self.at += 1;
            match byte {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let escape = *self.text.get(self.at)?;
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&first) && self.text[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                                let second = self.hex4()?;
                                if (0xdc00..0xe000).contains(&second) { 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00) } else { 0xfffd }
                            } else {
                                first
                            };
                            char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER)
                        }
                        _ => return None,
                    };
                    out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                _ => out.push(byte),
            }
        }
    }

    fn skip_value(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.string().map(drop),
            b'{' => self.object(|json, _| json.skip_value()),
            b'[' => {
                self.at += 1;
                if self.eat(b']') {
                    return Some(());
                }
                loop {
                    self.skip_value()?;
                    if self.eat(b']') {
                        return Some(());
                    }
                    self.eat(b',').then_some(())?;
                }
            }
            b't' => self.word(b"true"),
            b'f' => self.word(b"false"),
            b'n' => self.word(b"null"),
            _ => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(|b| b"+-.eE0123456789".contains(b)) {
                    self.at += 1;
                }
                (self.at > start).then_some(())
            }
        }
    }

    fn object(&mut self, mut field: impl FnMut(&mut Self, String) -> Option<()>) -> Option<()> {
        if !self.eat(b'{') {
            return None;
        }
        if self.eat(b'}') {
            return Some(());
        }
        loop {
            let key = self.string()?;
            self.eat(b':').then_some(())?;
            field(self, key)?;
            if self.eat(b'}') {
                return Some(());
            }
            self.eat(b',').then_some(())?;
        }
    }

    fn nullable_string(&mut self) -> Option<Option<String>> {
        if self.peek()? == b'n' {
            return self.word(b"null").map(|()| None);
        }
        self.string().map(Some)
    }
}

pub fn parse_sample(text: &str) -> Option<Vec<Sample>> {
    let mut json = Json { text: text.as_bytes(), at: 0 };
    if !json.eat(b'[') {
        return None;
    }
    let mut samples = Vec::new();
    if !json.eat(b']') {
        loop {
            let mut sample = Sample::default();
            if json.word(b"null").is_none() {
                json.object(|json, key| {
                    let slot = match key.to_ascii_lowercase().as_str() {
                        "device_name" => {
                            if let Some(name) = json.nullable_string()? {
                                sample.name = name;
                            }
                            return Some(());
                        }
                        "temp" => &mut sample.temp,
                        "power_draw" => &mut sample.power,
                        "gpu_util" => &mut sample.util,
                        "mem_total" => &mut sample.mem_total,
                        "mem_used" => &mut sample.mem_used,
                        _ => return json.skip_value(),
                    };
                    *slot = json.nullable_string()?;
                    Some(())
                })?;
            }
            samples.push(sample);
            if json.eat(b']') {
                break;
            }
            json.eat(b',').then_some(())?;
        }
    }
    (json.peek().is_none()).then_some(samples)
}

/// `85C`, `12.5W` and `40%` are numbers with units; anything unreadable is zero.
fn number(raw: &str) -> f64 {
    let mut value = raw.trim();
    for unit in ['C', 'W', '%'] {
        value = value.strip_suffix(unit).unwrap_or(value);
    }
    value.parse().unwrap_or(0.0)
}

/// Running totals of one device since the agent started.
#[derive(Debug, Default)]
struct Device {
    name: String,
    temperature: f64,
    mem_used: f64,
    mem_total: f64,
    usage: f64,
    power: f64,
    count: u32,
}

struct Snapshot {
    count: u32,
    usage: f64,
    power: f64,
}

pub struct Gpus {
    path: PathBuf,
    modified: Option<SystemTime>,
    devices: Vec<Device>,
    last_average: HashMap<usize, Gpu>,
    snapshots: HashMap<u16, HashMap<usize, Snapshot>>,
}

impl Gpus {
    pub fn new(path: PathBuf) -> Gpus {
        Gpus { path, modified: None, devices: Vec::new(), last_average: HashMap::new(), snapshots: HashMap::new() }
    }

    /// Each new version of the file is one more sample, as each line of nvtop's stream is.
    fn refresh(&mut self) {
        let Ok(modified) = std::fs::metadata(&self.path).and_then(|meta| meta.modified()) else { return };
        if self.modified == Some(modified) {
            return;
        }
        self.modified = Some(modified);
        if let Some(samples) = std::fs::read_to_string(&self.path).ok().and_then(|text| parse_sample(&text)) {
            self.add(samples);
        }
    }

    fn add(&mut self, samples: Vec<Sample>) {
        for (i, sample) in samples.into_iter().enumerate() {
            if self.devices.len() <= i {
                self.devices.push(Device::default());
            }
            let device = &mut self.devices[i];
            device.name = if sample.name.is_empty() { "GPU".to_owned() } else { sample.name };
            if let Some(temp) = sample.temp {
                device.temperature = number(&temp);
            }
            if let Some(used) = sample.mem_used {
                device.mem_used = megabytes(number(&used));
            }
            if let Some(total) = sample.mem_total {
                device.mem_total = megabytes(number(&total));
            }
            if let Some(util) = sample.util {
                device.usage += number(&util);
            }
            if let Some(power) = sample.power {
                device.power += number(&power);
            }
            device.count = device.count.wrapping_add(1);
        }
    }

    /// Each device's average since the previous request of `interval`, by its id (`n0`, `n1`, ...).
    pub fn current(&mut self, interval: u16) -> Vec<(String, Gpu)> {
        self.refresh();
        let snapshots = self.snapshots.entry(interval).or_default();
        let mut out = Vec::with_capacity(self.devices.len());
        for (i, device) in self.devices.iter().enumerate() {
            let last = snapshots.get(&i);
            let delta = device.count.wrapping_sub(last.map_or(0, |s| s.count));
            let mut average = if delta == 0 {
                if device.temperature == 0.0 && device.mem_used == 0.0 {
                    Gpu { name: device.name.clone(), ..Gpu::default() }
                } else {
                    let mut last_average = self.last_average.get(&i).cloned().unwrap_or_default();
                    if last_average.name.is_empty() {
                        last_average.name.clone_from(&device.name);
                    }
                    last_average
                }
            } else {
                let (usage, power) = last.map_or((device.usage, device.power), |s| (device.usage - s.usage, device.power - s.power));
                let average = Gpu {
                    name: device.name.clone(),
                    temperature: device.temperature,
                    mem_used: device.mem_used,
                    mem_total: device.mem_total,
                    usage: two_decimals(usage / f64::from(delta)),
                    power: two_decimals(power / f64::from(delta)),
                };
                self.last_average.insert(i, average.clone());
                average
            };
            average.temperature = two_decimals(device.temperature);
            average.mem_used = two_decimals(device.mem_used);
            average.mem_total = two_decimals(device.mem_total);
            snapshots.insert(i, Snapshot { count: device.count, usage: device.usage, power: device.power });
            let id = format!("n{i}");
            if self.devices.iter().filter(|d| d.name == device.name).count() > 1 {
                average.name = format!("{} {id}", device.name);
            }
            out.push((id, average));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nvtop_samples() {
        let samples = parse_sample(
            r#" [{"device_name":"Mali-G52 \"MC1\"","gpu_clock":"800MHz","processes":[{"pid":"1"}],"temp":null,
                 "gpu_util":"40%","power_draw":"2.5W","mem_used":"1048576","mem_total":"2097152","fan_speed":3}, null] "#,
        )
        .unwrap();
        assert_eq!(
            samples[0],
            Sample {
                name: "Mali-G52 \"MC1\"".into(),
                temp: None,
                power: Some("2.5W".into()),
                util: Some("40%".into()),
                mem_total: Some("2097152".into()),
                mem_used: Some("1048576".into()),
            }
        );
        assert_eq!(samples[1], Sample::default());
        assert_eq!(parse_sample(r#"[{"temp":45}]"#), None);
        assert_eq!(parse_sample(r#"[{"temp":"45C"}"#), None);
        assert_eq!(number(" 45C "), 45.0);
        assert_eq!(number("n/a"), 0.0);
    }

    #[test]
    fn averages_per_interval() {
        let mut gpus = Gpus::new(PathBuf::new());
        let sample = |util: &str| Sample { name: "Mali".into(), util: Some(util.into()), mem_used: Some("1048576".into()), ..Sample::default() };
        gpus.add(vec![sample("40%")]);
        gpus.add(vec![sample("60%")]);
        let current = gpus.current(1000);
        assert_eq!(current[0].0, "n0");
        assert_eq!((current[0].1.usage, current[0].1.mem_used), (50.0, 1.0));

        // No new sample since this interval's last request: the last average again.
        assert_eq!(gpus.current(1000)[0].1.usage, 50.0);
        gpus.add(vec![sample("10%")]);
        assert_eq!(gpus.current(1000)[0].1.usage, 10.0);
        assert_eq!(gpus.current(60_000)[0].1.usage, 36.67);
    }
}
