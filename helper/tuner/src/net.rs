//! The networks a listener answers.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

/// An IPv4 network, `192.168.0.0/16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    pub net: Ipv4Addr,
    pub prefix: u8,
}

impl Cidr {
    /// Parses `a.b.c.d/n` (`n` 0..=32); host bits of the address are masked off.
    pub fn parse(text: &str) -> Option<Cidr> {
        let (addr, prefix) = text.split_once('/')?;
        if prefix.is_empty() || prefix.len() > 2 || !prefix.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let prefix: u8 = prefix.parse().ok()?;
        if prefix > 32 {
            return None;
        }
        let addr: Ipv4Addr = addr.parse().ok()?;
        Some(Cidr {
            net: Ipv4Addr::from(u32::from(addr) & Self::mask(prefix)),
            prefix,
        })
    }

    fn mask(prefix: u8) -> u32 {
        if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(prefix.min(32)))
        }
    }

    /// Whether `ip` is inside the network. IPv4 only; an IPv4-mapped IPv6 address counts as its IPv4 address.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let v4 = match ip.to_canonical() {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => return false,
        };
        if self.prefix > 32 {
            return false;
        }
        let mask = Self::mask(self.prefix);
        u32::from(v4) & mask == u32::from(self.net) & mask
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.net, self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn cidr_parsing() {
        let ok = |text: &str| Cidr::parse(text).map(|c| c.to_string());
        assert_eq!(ok("192.168.0.0/16").as_deref(), Some("192.168.0.0/16"));
        assert_eq!(ok("10.0.0.0/8").as_deref(), Some("10.0.0.0/8"));
        assert_eq!(ok("192.168.1.77/24").as_deref(), Some("192.168.1.0/24"));
        assert_eq!(ok("0.0.0.0/0").as_deref(), Some("0.0.0.0/0"));
        assert_eq!(ok("1.2.3.4/32").as_deref(), Some("1.2.3.4/32"));
        for bad in [
            "",
            "192.168.0.0",
            "192.168.0.0/",
            "192.168.0.0/33",
            "192.168.0.0/-1",
            "192.168.0.0/+8",
            "192.168.0.0/008",
            "192.168.0/24",
            "192.168.0.256/24",
            "::1/128",
            "/8",
            "a.b.c.d/8",
            "192.168.0.0/8/8",
            "192.168.0.0/ 8",
        ] {
            assert_eq!(Cidr::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn cidr_membership() {
        let lan = Cidr::parse("192.168.0.0/16").unwrap();
        for yes in ["192.168.0.0", "192.168.1.5", "192.168.255.255"] {
            assert!(lan.contains(ip(yes)), "{yes}");
        }
        for no in [
            "192.167.255.255",
            "192.169.0.0",
            "10.0.0.1",
            "8.8.8.8",
            "::1",
        ] {
            assert!(!lan.contains(ip(no)), "{no}");
        }
        assert!(lan.contains(ip("::ffff:192.168.1.5")));
        assert!(!lan.contains(ip("::ffff:8.8.8.8")));
        assert!(!lan.contains(ip("fe80::1")));
        let all = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(all.contains(ip("8.8.8.8")));
        let one = Cidr::parse("10.1.2.3/32").unwrap();
        assert!(one.contains(ip("10.1.2.3")) && !one.contains(ip("10.1.2.4")));
    }
}
