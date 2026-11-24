//! Source-bound downloads: the wired route carries requests, each source address receives its own replies.
//! No bonding, policy routing, transparent proxy or application dependencies.

pub mod arp;
pub mod linux;
mod journal;
pub use journal::{FileJournal, Journal, JournalStore, Transaction};

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

pub type Result<T, E = String> = std::result::Result<T, E>;
pub const VALID_FOR_MS: u64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind { Ethernet, UsbEthernet, Wifi }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Link {
    pub interface: String,
    pub kind: Kind,
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub carrier_changes: u64,
    pub nominal_mbps: Option<u32>,
    pub signal_dbm: Option<i32>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

impl Link {
    pub fn same_network(&self, other: &Self) -> bool {
        if self.prefix == 0 || self.prefix > 32 || self.prefix != other.prefix { return false; }
        let mask = u32::MAX << (32 - self.prefix);
        u32::from(self.address) & mask == u32::from(other.address) & mask
    }

    pub fn healthy(&self) -> bool {
        self.address.is_private() && self.prefix > 0 && self.prefix < 32
            && valid_interface(&self.interface)
            && (self.kind != Kind::Wifi || self.signal_dbm.is_some_and(|s| (-75..=-1).contains(&s)))
    }
}

/// A route this engine may borrow: only private /32 routes via the existing wired gateway.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    pub destination: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub interface: String,
    pub source: Option<Ipv4Addr>,
    pub metric: u32,
    pub protocol: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub links: Vec<Link>,
    pub default_interface: String,
    pub gateway: Ipv4Addr,
    pub host_routes: Vec<Route>,
    pub all_arp_ignore: u32,
    pub all_arp_announce: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Knob { ArpIgnore, ArpAnnounce, RpFilter }
impl Knob {
    pub fn name(self) -> &'static str {
        match self { Self::ArpIgnore => "arp_ignore", Self::ArpAnnounce => "arp_announce", Self::RpFilter => "rp_filter" }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Key {
    Sysctl { interface: String, knob: Knob },
    HostRoute { destination: Ipv4Addr },
    WifiPower,
    WifiProfile,
}
impl Key {
    pub fn radio(&self) -> bool { matches!(self, Self::WifiPower | Self::WifiProfile) }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase", deny_unknown_fields)]
pub enum Value { Number(u32), Route(Option<Route>) }

pub trait Device {
    /// Forward changes require the owner's current permission; rollback is always allowed.
    fn allowed(&self) -> bool { true }
    fn read(&self, key: &Key) -> Result<Value>;
    fn write(&self, key: &Key, value: &Value) -> Result<()>;
}

pub fn valid_interface(name: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    name.len() < 16 && (name == "wlan0"
        || name.strip_prefix("eth").is_some_and(digits)
        || name.strip_prefix("usb").is_some_and(digits)
        || name.strip_prefix("enx").is_some_and(|s| s.len() == 12 && s.bytes().all(|c| c.is_ascii_hexdigit())))
}

pub(crate) fn validate(key: &Key, value: &Value) -> Result<()> {
    let valid = match (key, value) {
        (Key::Sysctl { interface, knob }, Value::Number(n)) => valid_interface(interface) && *n <= match knob { Knob::ArpIgnore => 8, Knob::ArpAnnounce | Knob::RpFilter => 2 },
        (Key::WifiPower, Value::Number(n)) => *n <= 1,
        (Key::WifiProfile, Value::Number(_)) => true,
        (Key::HostRoute { destination }, Value::Route(route)) => destination.is_private() && route.as_ref().is_none_or(|r| {
            r.destination == *destination && r.gateway.is_private() && valid_interface(&r.interface)
                && r.source.is_none_or(|s| s.is_private())
                && r.protocol.as_ref().is_none_or(|s| !s.is_empty() && s.len() <= 16 && s.bytes().all(|c| c.is_ascii_alphanumeric()))
        }),
        _ => false,
    };
    if valid { Ok(()) } else { Err("Invalid network rollback record".into()) }
}

#[derive(Debug)]
pub struct Plan {
    pub control: String,
    pub candidates: Vec<Link>,
    pub settings: Vec<(Key, Value)>,
}

pub fn plan(snapshot: &Snapshot, wifi_allowed: bool) -> Result<Plan> {
    let wire = snapshot.links.iter().find(|l| l.interface == snapshot.default_interface && l.kind != Kind::Wifi && l.healthy())
        .ok_or("Waiting for a healthy wired default route")?;
    if !snapshot.gateway.is_private() || snapshot.all_arp_ignore > 1 || snapshot.all_arp_announce > 2 {
        return Err("Network rules do not support separate return paths".into());
    }
    let mut candidates: Vec<_> = snapshot.links.iter().filter(|l| l.healthy() && l.same_network(wire)
        && (l.kind != Kind::Wifi || wifi_allowed)).cloned().collect();
    candidates.sort_by_key(|l| std::cmp::Reverse(l.nominal_mbps.unwrap_or(0)));
    let mut settings = Vec::with_capacity(candidates.len() * 3 + snapshot.host_routes.len());
    if candidates.len() > 1 {
        for link in &candidates {
            for (knob, n) in [(Knob::ArpIgnore, 1), (Knob::ArpAnnounce, 2), (Knob::RpFilter, 2)] {
                settings.push((Key::Sysctl { interface: link.interface.clone(), knob }, Value::Number(n)));
            }
        }
    }
    // A prior destination-only Wi-Fi workaround must not send Plex controls/ACKs over the radio.
    // Only private /32 routes using this same gateway are borrowed; connected, default and P2P routes stay LG's.
    for route in &snapshot.host_routes {
        if route.interface == "wlan0" && route.gateway == snapshot.gateway && route.destination.is_private() {
            let mut wired = route.clone();
            wired.interface.clone_from(&wire.interface);
            wired.source = None;
            settings.push((Key::HostRoute { destination: route.destination }, Value::Route(Some(wired))));
        }
    }
    Ok(Plan { control: wire.interface.clone(), candidates, settings })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rate {
    pub interface: String,
    pub rx_mbps: f64,
    pub tx_mbps: f64,
    pub peak_rx_mbps: f64,
}

#[derive(Default)]
pub struct Meter { previous: Option<(u64, Vec<Link>)>, rates: Vec<Rate> }
impl Meter {
    /// Passive counters, not a speed test. Address/association changes and counter resets start a new sample.
    pub fn sample(&mut self, now_ms: u64, links: &[Link]) -> &[Rate] {
        if let Some((then, old)) = self.previous.as_ref().filter(|(t, _)| now_ms > *t && now_ms - *t <= 5_000) {
            self.rates.retain(|r| links.iter().any(|l| l.interface == r.interface));
            for link in links {
                let rate = if let Some(index) = self.rates.iter().position(|r| r.interface == link.interface) {
                    &mut self.rates[index]
                } else {
                    self.rates.push(Rate { interface: link.interface.clone(), rx_mbps: 0.0, tx_mbps: 0.0, peak_rx_mbps: 0.0 });
                    self.rates.last_mut().unwrap()
                };
                let previous = old.iter().find(|p| p.interface == link.interface && p.address == link.address && p.carrier_changes == link.carrier_changes);
                match previous.filter(|p| link.rx_bytes >= p.rx_bytes && link.tx_bytes >= p.tx_bytes) {
                    Some(p) => {
                        let factor = 0.008 / (now_ms - then) as f64;
                        rate.rx_mbps = ((link.rx_bytes - p.rx_bytes) as f64 * factor * 10.0).round() / 10.0;
                        rate.tx_mbps = ((link.tx_bytes - p.tx_bytes) as f64 * factor * 10.0).round() / 10.0;
                        rate.peak_rx_mbps = rate.peak_rx_mbps.max(rate.rx_mbps);
                    }
                    None => { rate.rx_mbps = 0.0; rate.tx_mbps = 0.0; rate.peak_rx_mbps = 0.0; }
                }
            }
        } else { self.rates.clear(); }
        self.previous = Some((now_ms, links.to_vec()));
        &self.rates
    }
}

#[cfg(test)]
mod tests;
