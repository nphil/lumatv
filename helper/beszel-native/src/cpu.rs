//! CPU use from `/proc/stat`, measured as the Go agent measures it (`agent/cpu.go`): each request
//! interval against the previous request of the same interval, a new interval against the default one.

use std::collections::HashMap;

use crate::agent::DEFAULT_INTERVAL;

/// Seconds spent in each state, from one `cpu` line.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Times {
    user: f64,
    nice: f64,
    system: f64,
    idle: f64,
    iowait: f64,
    irq: f64,
    softirq: f64,
    steal: f64,
    guest: f64,
    guest_nice: f64,
}

impl Times {
    fn parse(line: &str) -> Option<Times> {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 || !fields[0].starts_with("cpu") {
            return None;
        }
        // Clock ticks, 100 per second; absent trailing fields are older kernels.
        let field = |i: usize| fields.get(i).map_or(Some(0.0), |f| f.parse::<f64>().ok().map(|v| v / 100.0));
        Some(Times {
            user: field(1)?,
            nice: field(2)?,
            system: field(3)?,
            idle: field(4)?,
            iowait: field(5)?,
            irq: field(6)?,
            softirq: field(7)?,
            steal: field(8)?,
            guest: field(9)?,
            guest_nice: field(10)?,
        })
    }

    /// Total and busy time. Linux already counts guest time inside user and nice.
    fn all_busy(&self) -> (f64, f64) {
        let mut total = self.user
            + self.system
            + self.idle
            + self.nice
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
            + self.guest
            + self.guest_nice;
        total -= self.guest;
        total -= self.guest_nice;
        (total, total - self.idle - self.iowait)
    }
}

/// `/proc/stat`'s aggregate line and one line per online core.
pub struct Stat {
    total: Times,
    cores: Vec<Times>,
}

pub fn parse_stat(text: &str) -> Option<Stat> {
    let mut lines = text.lines();
    let total = Times::parse(lines.next()?)?;
    let cores = lines.take_while(|line| line.starts_with("cpu")).filter_map(Times::parse).collect();
    Some(Stat { total, cores })
}

fn clamp_percent(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn busy_percent(t1: &Times, t2: &Times) -> f64 {
    let (t1_all, t1_busy) = t1.all_busy();
    let (t2_all, t2_busy) = t2.all_busy();
    if t2_all <= t1_all || t2_busy <= t1_busy {
        return 0.0;
    }
    clamp_percent((t2_busy - t1_busy) / (t2_all - t1_all) * 100.0)
}

#[derive(Debug, Default, PartialEq)]
pub struct Usage {
    pub total: f64,
    /// user, system, iowait, steal, idle, irq, softirq, nice.
    pub breakdown: [f64; 8],
}

#[derive(Default)]
pub struct Cpu {
    last: HashMap<u16, Times>,
    last_cores: HashMap<u16, Vec<Times>>,
}

impl Cpu {
    /// Starts the default interval from `stat`, so the first request already has a span to measure.
    pub fn new(stat: Option<Stat>) -> Cpu {
        let mut cpu = Cpu::default();
        if let Some(stat) = stat {
            cpu.last.insert(DEFAULT_INTERVAL, stat.total);
            if !stat.cores.is_empty() {
                cpu.last_cores.insert(DEFAULT_INTERVAL, stat.cores);
            }
        }
        cpu
    }

    /// The use since the previous sample of `interval`, and each core's use rounded to a percent.
    pub fn sample(&mut self, interval: u16, stat: Stat) -> (Usage, Vec<u8>) {
        let usage = self.usage(interval, stat.total);
        let cores = if stat.cores.is_empty() { Vec::new() } else { self.cores(interval, stat.cores) };
        (usage, cores)
    }

    fn usage(&mut self, interval: u16, t2: Times) -> Usage {
        let seed = self.last.get(&DEFAULT_INTERVAL).copied().unwrap_or_default();
        let t1 = self.last.insert(interval, t2).unwrap_or(seed);
        let total_delta = t2.all_busy().0 - t1.all_busy().0;
        if total_delta <= 0.0 {
            return Usage::default();
        }
        let share = |before: f64, after: f64| clamp_percent((after - before) / total_delta * 100.0);
        Usage {
            total: busy_percent(&t1, &t2),
            breakdown: [
                share(t1.user, t2.user),
                share(t1.system, t2.system),
                share(t1.iowait, t2.iowait),
                share(t1.steal, t2.steal),
                share(t1.idle, t2.idle),
                share(t1.irq, t2.irq),
                share(t1.softirq, t2.softirq),
                share(t1.nice, t2.nice),
            ],
        }
    }

    fn cores(&mut self, interval: u16, current: Vec<Times>) -> Vec<u8> {
        let last = match self.last_cores.remove(&interval) {
            Some(last) => last,
            None => self.last_cores.get(&DEFAULT_INTERVAL).cloned().unwrap_or_default(),
        };
        let usage = last.iter().zip(&current).map(|(t1, t2)| busy_percent(t1, t2).round() as u8).collect();
        self.last_cores.insert(interval, current);
        usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(user: u64, idle: u64, core_user: [u64; 2]) -> Stat {
        let text = format!(
            "cpu  {user} 0 0 {idle} 0 0 0 0 0 0\ncpu0 {} 0 0 50 0 0 0 0\ncpu1 {} 0 0 50 0 0 0 0\nintr 1\n",
            core_user[0], core_user[1]
        );
        parse_stat(&text).unwrap()
    }

    #[test]
    fn measures_each_interval_from_its_own_last_sample() {
        let mut cpu = Cpu::new(Some(stat(100, 100, [10, 10])));
        // A new interval starts from the default interval's sample.
        let (usage, cores) = cpu.sample(1000, stat(150, 150, [60, 10]));
        assert_eq!(usage.total, 50.0);
        assert_eq!(usage.breakdown[0], 50.0);
        assert_eq!(usage.breakdown[4], 50.0);
        assert_eq!(cores, [100, 0]);

        let (usage, _) = cpu.sample(1000, stat(150, 250, [60, 10]));
        assert_eq!(usage.total, 0.0);
        // The default interval still measures from the start.
        let (usage, _) = cpu.sample(DEFAULT_INTERVAL, stat(150, 250, [60, 10]));
        assert_eq!(usage.total, 25.0);
        // No time passed: no reading.
        assert_eq!(cpu.sample(DEFAULT_INTERVAL, stat(150, 250, [60, 10])).0, Usage::default());
    }
}
