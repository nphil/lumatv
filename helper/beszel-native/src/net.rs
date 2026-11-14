//! Network traffic from `/proc/net/dev` for the interfaces `NICS` selects (`agent/network.go`).

use std::collections::HashMap;
use std::time::Instant;

use crate::filter::NameFilter;
use crate::slog;
use crate::system::Stats;

pub struct Counters {
    pub name: String,
    pub sent: u64,
    pub recv: u64,
}

/// gopsutil's reader: a malformed number fails the whole read.
pub fn parse_net_dev(text: &str) -> Option<Vec<Counters>> {
    let mut out = Vec::new();
    for line in text.lines().skip(2) {
        let Some((name, data)) = line.rsplit_once(':') else { continue };
        let name = name.trim();
        let fields: Vec<&str> = data.split_whitespace().collect();
        if name.is_empty() || fields.len() < 13 {
            continue;
        }
        let mut values = [0u64; 13];
        for (i, (value, field)) in values.iter_mut().zip(&fields).enumerate() {
            // Fields 5 to 7 are receive compression, frame and multicast counts, which gopsutil skips.
            if !(5..=7).contains(&i) {
                *value = field.parse().ok()?;
            }
        }
        out.push(Counters { name: name.to_owned(), recv: values[0], sent: values[8] });
    }
    Some(out)
}

fn read_counters() -> Option<Vec<Counters>> {
    parse_net_dev(&std::fs::read_to_string("/proc/net/dev").ok()?)
}

/// Rates above this are a counter reset, not traffic.
const MAX_RATE: u64 = 10_000_000_000;

pub struct Net {
    filter: Option<NameFilter>,
    tracked: Vec<String>,
    /// Per interval: when it was last measured and each interface's `[sent, recv]` then.
    last: HashMap<u16, (Instant, HashMap<String, [u64; 2]>)>,
}

impl Net {
    /// `nics` is `NICS`, when set.
    pub fn new(nics: Option<&str>) -> Net {
        let mut net = Net { filter: nics.map(NameFilter::parse), tracked: Vec::new(), last: HashMap::new() };
        net.detect(read_counters().as_deref().unwrap_or_default());
        net
    }

    fn skip(&self, c: &Counters) -> bool {
        if let Some(filter) = &self.filter {
            if !filter.allows(&c.name, filter.is_blacklist()) {
                return true;
            }
            // Named interfaces are kept even while idle.
            if !filter.is_blacklist() {
                return false;
            }
        }
        ["lo", "docker", "br-", "veth", "bond", "cali"].iter().any(|prefix| c.name.starts_with(prefix)) || c.recv == 0 || c.sent == 0
    }

    fn detect(&mut self, counters: &[Counters]) {
        self.tracked.clear();
        for c in counters {
            if !self.skip(c) {
                slog::info("Detected network interface", &[("name", &c.name), ("sent", &c.sent), ("recv", &c.recv)]);
                self.tracked.push(c.name.clone());
            }
        }
        self.last.clear();
    }

    pub fn update(&mut self, interval: u16, stats: &mut Stats) {
        if self.tracked.is_empty() {
            self.detect(read_counters().as_deref().unwrap_or_default());
        }
        let Some(counters) = read_counters() else { return };
        self.measure(interval, stats, &counters, Instant::now());
    }

    fn measure(&mut self, interval: u16, stats: &mut Stats, counters: &[Counters], now: Instant) {
        let previous = self.last.remove(&interval);
        let ms = previous.as_ref().map_or(0, |(at, _)| now.saturating_duration_since(*at).as_millis() as u64);
        let mut current = HashMap::new();
        let (mut up, mut down) = (0u64, 0u64);
        for c in counters.iter().filter(|c| self.tracked.contains(&c.name)) {
            let before = previous.as_ref().and_then(|(_, values)| values.get(&c.name));
            let rate = |now: u64, before: Option<u64>| match before {
                // A counter that went down restarted from zero.
                Some(before) if ms > 0 => (if now >= before { now - before } else { now }).wrapping_mul(1000) / ms,
                _ => 0,
            };
            let (sent, recv) = (rate(c.sent, before.map(|v| v[0])), rate(c.recv, before.map(|v| v[1])));
            current.insert(c.name.clone(), [c.sent, c.recv]);
            stats.interfaces.push((c.name.clone(), [sent, recv, c.sent, c.recv]));
            up += sent;
            down += recv;
        }
        if up > MAX_RATE || down > MAX_RATE {
            slog::warn("Invalid net stats. Resetting.", &[("sent", &up), ("recv", &down)]);
            for c in counters.iter().filter(|c| self.tracked.contains(&c.name)) {
                slog::info(&c.name, &[("recv", &c.recv), ("sent", &c.sent)]);
            }
            self.detect(counters);
            stats.bandwidth = [0, 0];
            return;
        }
        stats.bandwidth = [up, down];
        self.last.insert(interval, (now, current));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const NET_DEV: &str = "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes\n\
                           lo: 500 5 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n\
                           eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n\
                           wlan0: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";

    fn counters(eth0: [u64; 2]) -> Vec<Counters> {
        let mut c = parse_net_dev(NET_DEV).unwrap();
        c[1].recv = eth0[1];
        c[1].sent = eth0[0];
        c
    }

    #[test]
    fn tracks_the_named_interfaces() {
        let c = parse_net_dev(NET_DEV).unwrap();
        assert_eq!((c[1].name.as_str(), c[1].recv, c[1].sent), ("eth0", 1000, 2000));
        assert!(parse_net_dev("a\nb\neth0: x 0 0 0 0 0 0 0 0 0 0 0 0\n").is_none());

        let mut named = Net { filter: Some(NameFilter::parse("eth0,wlan0")), tracked: Vec::new(), last: HashMap::new() };
        named.detect(&c);
        assert_eq!(named.tracked, ["eth0", "wlan0"]);
        // Without NICS, loopback and idle interfaces are left out.
        let mut any = Net { filter: None, tracked: Vec::new(), last: HashMap::new() };
        any.detect(&c);
        assert_eq!(any.tracked, ["eth0"]);
    }

    #[test]
    fn measures_each_interval_from_its_last_request() {
        let mut net = Net { filter: None, tracked: vec!["eth0".into()], last: HashMap::new() };
        let start = Instant::now();
        let mut stats = Stats::default();
        net.measure(1000, &mut stats, &counters([2000, 1000]), start);
        assert_eq!(stats.bandwidth, [0, 0]);
        assert_eq!(stats.interfaces, [("eth0".to_owned(), [0, 0, 2000, 1000])]);

        let mut stats = Stats::default();
        net.measure(1000, &mut stats, &counters([4000, 1500]), start + Duration::from_millis(500));
        assert_eq!(stats.bandwidth, [4000, 1000]);

        // A reset counter counts from zero; an impossible rate starts over.
        let mut stats = Stats::default();
        net.measure(1000, &mut stats, &counters([100, 20_000_001_500]), start + Duration::from_millis(1500));
        assert_eq!(stats.bandwidth, [0, 0]);
        assert!(net.last.is_empty());
    }
}
