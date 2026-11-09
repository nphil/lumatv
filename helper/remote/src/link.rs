//! What a phone finds the TV by: the pairing link a QR code carries, and the mDNS TXT record.

use crate::{VERSION, hex, unhex};

/// `lumaremote://pair?h=<address>&p=<port>&id=<TV id, 16 hex>&c=<code without the dash>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairLink {
    pub host: String,
    pub port: u16,
    pub tv: [u8; 8],
    pub code: String,
}

impl PairLink {
    pub fn to_url(&self) -> String {
        format!("lumaremote://pair?h={}&p={}&id={}&c={}", self.host, self.port, hex(&self.tv), self.code)
    }

    /// A scanned or tapped link; the code is normalized as a typed one is.
    pub fn parse(url: &str) -> Option<PairLink> {
        let query = url.strip_prefix("lumaremote://pair?")?;
        let (mut host, mut port, mut tv, mut code) = (None, None, None, None);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=')?;
            match key {
                "h" if !value.is_empty() => host = Some(value.to_string()),
                "p" => port = value.parse().ok(),
                "id" => tv = unhex::<8>(value),
                "c" => code = crate::pairing::normalize(value),
                _ => {}
            }
        }
        Some(PairLink { host: host?, port: port?, tv: tv?, code: code? })
    }
}

/// The TXT record: `v`, `id`, `name`, `mac` (12 hex, for Wake-on-LAN) when known, and `pair` 1 while a pairing window is open.
pub fn txt(tv: &[u8; 8], name: &str, mac: Option<[u8; 6]>, pairing: bool) -> Vec<(String, String)> {
    let mut txt = vec![("v".to_string(), VERSION.to_string()), ("id".to_string(), hex(tv)), ("name".to_string(), name.to_string())];
    if let Some(mac) = mac {
        txt.push(("mac".to_string(), hex(&mac)));
    }
    txt.push(("pair".to_string(), if pairing { "1" } else { "0" }.to_string()));
    txt
}

/// `aa:bb:cc:dd:ee:ff`, as `welcome` and `paired` give it.
pub fn mac_text(mac: [u8; 6]) -> String {
    mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip_and_a_bad_one_is_refused() {
        let link = PairLink { host: "192.168.1.242".into(), port: 18733, tv: [0x0a, 1, 2, 3, 4, 5, 6, 0xff], code: "K7WDX2AB".into() };
        let url = link.to_url();
        assert_eq!(url, "lumaremote://pair?h=192.168.1.242&p=18733&id=0a010203040506ff&c=K7WDX2AB");
        assert_eq!(PairLink::parse(&url), Some(link.clone()));
        assert_eq!(PairLink::parse(&url.replace("c=K7WDX2AB", "c=k7wd-x2ab")), Some(link));
        assert_eq!(PairLink::parse(&url.replace("c=K7WDX2AB", "c=K7WDX2A")), None);
        assert_eq!(PairLink::parse("lumaremote://pair?h=x&p=1"), None);
        assert_eq!(mac_text([0x74, 0xc1, 0x7e, 0xbd, 0xae, 0x30]), "74:c1:7e:bd:ae:30");
    }
}
