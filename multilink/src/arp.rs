//! ARP flux guard. Two interfaces in one subnet answer ARP for each other's address
//! (arp_ignore=0), so the gateway sees one IP behind two MACs. Core, independent of Multi Link.
use crate::Result;
use std::collections::BTreeSet;
use std::ffi::CStr;
use std::fs;

pub const IGNORE: &str = "1";
pub const ANNOUNCE: &str = "2";

/// Interfaces that share an IPv4 subnet with another interface. Input: (name, address, prefix).
pub fn overlapping(addrs: &[(String, u32, u8)]) -> BTreeSet<String> {
    let net = |a: u32, p: u8| if p == 0 { 0 } else { a & (u32::MAX << (32 - u32::from(p.min(32)))) };
    let mut out = BTreeSet::new();
    for (i, (ni, ai, pi)) in addrs.iter().enumerate() {
        for (nj, aj, pj) in &addrs[i + 1..] {
            let p = *pi.min(pj);
            if ni != nj && p > 0 && net(*ai, p) == net(*aj, p) {
                out.insert(ni.clone());
                out.insert(nj.clone());
            }
        }
    }
    out
}

fn addresses() -> Result<Vec<(String, u32, u8)>> {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list freed below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 { return Err(std::io::Error::last_os_error().to_string()); }
    let mut out = Vec::new();
    let mut p = head;
    while !p.is_null() {
        let a = unsafe { &*p };
        p = a.ifa_next;
        if a.ifa_addr.is_null() || a.ifa_netmask.is_null() || a.ifa_flags & libc::IFF_UP as u32 == 0 || a.ifa_flags & libc::IFF_LOOPBACK as u32 != 0 { continue; }
        if unsafe { (*a.ifa_addr).sa_family } != libc::AF_INET as u16 { continue; }
        let name = unsafe { CStr::from_ptr(a.ifa_name) }.to_string_lossy().into_owned();
        let ip = u32::from_be(unsafe { &*a.ifa_addr.cast::<libc::sockaddr_in>() }.sin_addr.s_addr);
        let mask = u32::from_be(unsafe { &*a.ifa_netmask.cast::<libc::sockaddr_in>() }.sin_addr.s_addr);
        out.push((name, ip, mask.leading_ones() as u8));
    }
    unsafe { libc::freeifaddrs(head) };
    Ok(out)
}

fn set(dir: &str, knob: &str, want: &str) -> Result<bool> {
    let path = format!("/proc/sys/net/ipv4/conf/{dir}/{knob}");
    match fs::read_to_string(&path) {
        Ok(v) if v.trim() == want => return Ok(false),
        Ok(_) => {}
        Err(_) => return Ok(false), // interface vanished
    }
    fs::write(&path, want).map_err(|e| format!("{path}: {e}"))?;
    Ok(true)
}

/// Apply arp_ignore=1/arp_announce=2 (per interface and `all`) while any subnet is shared.
/// Returns the number of values changed.
pub fn apply() -> Result<usize> {
    let shared = overlapping(&addresses()?);
    if shared.is_empty() { return Ok(0); }
    let mut changed = 0;
    for dir in shared.iter().map(String::as_str).chain(["all"]) {
        if dir.contains(['/', '.']) { continue; }
        for (knob, want) in [("arp_ignore", IGNORE), ("arp_announce", ANNOUNCE)] {
            changed += usize::from(set(dir, knob, want)?);
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(n: &str, ip: [u8; 4], p: u8) -> (String, u32, u8) { (n.into(), u32::from_be_bytes(ip), p) }
    #[test]
    fn same_subnet_flagged_others_not() {
        let s = overlapping(&[a("eth0", [192, 168, 1, 242], 24), a("wlan0", [192, 168, 1, 180], 24), a("usb0", [10, 0, 0, 2], 24)]);
        assert_eq!(s.into_iter().collect::<Vec<_>>(), ["eth0", "wlan0"]);
        assert!(overlapping(&[a("eth0", [192, 168, 1, 2], 24), a("wlan0", [192, 168, 2, 2], 24)]).is_empty());
    }
}
