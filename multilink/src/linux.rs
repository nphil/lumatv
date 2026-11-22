//! Linux data and fixed, typed operations; no shell, route flush, interface-down or P2P operation.
use crate::{Device, Key, Kind, Link, Result, Route, Snapshot, Value, valid_interface, validate};
use std::ffi::CStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Linux;

fn number(path: impl AsRef<Path>) -> Result<u64> {
    fs::read_to_string(path).map_err(|e| e.to_string())?.trim().parse::<u64>().map_err(|e| e.to_string())
}

pub fn uptime_ms() -> Result<u64> {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: the pointer is writable for one timespec; only a successful call is read.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, time.as_mut_ptr()) } != 0 { return Err(std::io::Error::last_os_error().to_string()); }
    let time = unsafe { time.assume_init() };
    Ok((time.tv_sec as u64).saturating_mul(1_000) + time.tv_nsec as u64 / 1_000_000)
}

fn ip(args: &[String]) -> Result<String> {
    let output = Command::new("/sbin/ip").arg("-4").args(args).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

pub fn parse_host_route(line: &str) -> Option<Route> {
    let mut words = line.split_whitespace();
    let dest = words.next()?;
    let destination: Ipv4Addr = dest.strip_suffix("/32").unwrap_or(dest).parse().ok()?;
    if !destination.is_private() { return None; }
    let mut route = Route { destination, gateway: Ipv4Addr::UNSPECIFIED, interface: String::new(), source: None, metric: 0, protocol: None };
    while let Some(key) = words.next() {
        let value = words.next()?;
        match key {
            "via" => route.gateway = value.parse().ok()?,
            "dev" => route.interface = value.to_owned(),
            "src" => route.source = Some(value.parse().ok()?),
            "metric" => route.metric = value.parse().ok()?,
            "proto" => route.protocol = Some(value.to_owned()),
            _ => return None,
        }
    }
    validate(&Key::HostRoute { destination }, &Value::Route(Some(route.clone()))).ok()?;
    Some(route)
}

impl Linux {
    pub fn snapshot(&self) -> Result<Snapshot> {
        let wireless = fs::read_to_string("/proc/net/wireless").unwrap_or_default();
        let mut address = std::ptr::null_mut();
        // SAFETY: getifaddrs initializes the list; List frees it on every path.
        if unsafe { libc::getifaddrs(&mut address) } != 0 { return Err(std::io::Error::last_os_error().to_string()); }
        struct List(*mut libc::ifaddrs);
        impl Drop for List { fn drop(&mut self) { unsafe { libc::freeifaddrs(self.0); } } }
        let _list = List(address);
        let mut links = Vec::new();
        while !address.is_null() {
            let a = unsafe { &*address };
            address = a.ifa_next;
            if a.ifa_addr.is_null() || a.ifa_netmask.is_null() || a.ifa_flags & libc::IFF_UP as u32 == 0 { continue; }
            if unsafe { (*a.ifa_addr).sa_family } != libc::AF_INET as u16 { continue; }
            let interface = unsafe { CStr::from_ptr(a.ifa_name) }.to_string_lossy();
            if !valid_interface(&interface) { continue; }
            let base = format!("/sys/class/net/{interface}");
            if number(format!("{base}/carrier")).ok() != Some(1) { continue; }
            let ipv4 = unsafe { &*a.ifa_addr.cast::<libc::sockaddr_in>() };
            let netmask = unsafe { &*a.ifa_netmask.cast::<libc::sockaddr_in>() };
            let mask = u32::from_be(netmask.sin_addr.s_addr);
            let prefix = mask.leading_ones() as u8;
            if prefix == 0 || mask != u32::MAX << (32 - prefix) { continue; }
            let kind = if interface == "wlan0" { Kind::Wifi }
                else if fs::canonicalize(format!("{base}/device")).ok().is_some_and(|p| p.components().any(|c| c.as_os_str().to_string_lossy().starts_with("usb"))) { Kind::UsbEthernet }
                else { Kind::Ethernet };
            let signal_dbm = wireless.lines().find_map(|l| {
                let (name, values) = l.split_once(':')?;
                if name.trim() != interface { return None; }
                let raw: f64 = values.split_whitespace().nth(2)?.parse().ok()?;
                Some(if raw > 127.0 { raw as i32 - 256 } else { raw as i32 })
            });
            if links.iter().any(|l: &Link| l.interface == interface) { return Err("Multiple IPv4 addresses on one link".into()); }
            links.push(Link { interface: interface.into_owned(), kind, address: Ipv4Addr::from(ipv4.sin_addr.s_addr.to_ne_bytes()), prefix,
                carrier_changes: number(format!("{base}/carrier_changes"))?,
                nominal_mbps: number(format!("{base}/speed")).ok().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0), signal_dbm,
                rx_bytes: number(format!("{base}/statistics/rx_bytes"))?, tx_bytes: number(format!("{base}/statistics/tx_bytes"))? });
        }
        let table = fs::read_to_string("/proc/net/route").map_err(|e| e.to_string())?;
        let (default_interface, gateway) = default_route(&table).ok_or("No unambiguous default route")?;
        let mut host_routes = Vec::new();
        for line in table.lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 8 || fields[0] != "wlan0" || fields[7] != "FFFFFFFF" { continue; }
            let Some(destination) = hex_address(fields[1]).filter(Ipv4Addr::is_private) else { continue };
            if fields[2] == "00000000" { continue; }
            if let Value::Route(Some(route)) = self.read(&Key::HostRoute { destination })? { host_routes.push(route); }
        }
        Ok(Snapshot { links, default_interface, gateway, host_routes,
            all_arp_ignore: number("/proc/sys/net/ipv4/conf/all/arp_ignore")? as u32,
            all_arp_announce: number("/proc/sys/net/ipv4/conf/all/arp_announce")? as u32 })
    }
}

fn hex_address(s: &str) -> Option<Ipv4Addr> { Some(Ipv4Addr::from(u32::from_str_radix(s, 16).ok()?.to_le_bytes())) }

pub fn default_route(table: &str) -> Option<(String, Ipv4Addr)> {
    let mut best: Option<(u32, String, Ipv4Addr)> = None;
    let mut ambiguous = false;
    for line in table.lines().skip(1) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 8 || fields[1] != "00000000" || fields[7] != "00000000" { continue; }
        if u32::from_str_radix(fields[3], 16).ok()? & 3 != 3 { continue; }
        let metric: u32 = fields[6].parse().ok()?;
        let gateway = hex_address(fields[2])?;
        match best.as_ref() {
            Some((m, ..)) if *m == metric => ambiguous = true,
            Some((m, ..)) if *m < metric => {},
            _ => { best = Some((metric, fields[0].into(), gateway)); ambiguous = false; },
        }
    }
    if ambiguous { None } else { best.map(|(_, interface, gateway)| (interface, gateway)) }
}

impl Device for Linux {
    fn read(&self, key: &Key) -> Result<Value> {
        match key {
            Key::Sysctl { interface, knob } if valid_interface(interface) => {
                let path = format!("/proc/sys/net/ipv4/conf/{interface}/{}", knob.name());
                // Removed USB devices have no remaining sysctl to restore.
                if !Path::new(&path).exists() { return Ok(Value::Number(u32::MAX)); }
                Ok(Value::Number(number(path)? as u32))
            }
            Key::HostRoute { destination } if destination.is_private() => {
                let text = ip(&["route".into(), "show".into(), "exact".into(), format!("{destination}/32")])?;
                if text.trim().is_empty() { return Ok(Value::Route(None)); }
                if text.lines().count() != 1 { return Err("Ambiguous destination route".into()); }
                parse_host_route(&text).map(|r| Value::Route(Some(r))).ok_or_else(|| "Destination route cannot be borrowed safely".into())
            }
            _ => Err("Not a Linux network key".into()),
        }
    }
    fn write(&self, key: &Key, value: &Value) -> Result<()> {
        validate(key, value)?;
        match (key, value) {
            (Key::Sysctl { interface, knob }, Value::Number(n)) => fs::write(format!("/proc/sys/net/ipv4/conf/{interface}/{}", knob.name()), n.to_string()).map_err(|e| e.to_string()),
            (Key::HostRoute { destination }, Value::Route(route)) => {
                let mut args = vec!["route".into(), if route.is_some() { "replace" } else { "del" }.into(), format!("{destination}/32")];
                if let Some(r) = route {
                    args.extend(["via".into(), r.gateway.to_string(), "dev".into(), r.interface.clone(), "metric".into(), r.metric.to_string()]);
                    if let Some(source) = r.source { args.extend(["src".into(), source.to_string()]); }
                    if let Some(protocol) = &r.protocol { args.extend(["proto".into(), protocol.clone()]); }
                }
                ip(&args).map(|_| ())
            }
            _ => Err("Not a Linux network key".into()),
        }
    }
}
