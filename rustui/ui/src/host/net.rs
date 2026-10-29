//! The network as the quick panel shows it (`kit/Net.qml`): which link is up, the Wi-Fi name and strength, the address, from the
//! connection manager (`getStatus`, subscribed). What it does not say stays empty and the row shows nothing for it.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

pub const STATUS: &str = "luna://com.webos.service.connectionmanager/getStatus";
pub const TAG: u64 = 0x4E00_0001;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Net {
    pub wifi: bool,
    pub wired: bool,
    pub ssid: String,
    /// 0 to 4, from the signal level.
    pub bars: u8,
    pub ip: String,
    pub band: String,
    pub dbm: String,
    pub method: String,
}

impl Net {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe() -> Vec<Effect> {
        vec![Effect::Luna { tag: TAG, uri: STATUS.into(), payload: json!({"subscribe": true}), subscribe: true }]
    }

    pub fn strength(&self) -> &'static str {
        if !self.wifi {
            ""
        } else {
            match self.bars {
                3.. => "Strong",
                2 => "Good",
                1 => "Weak",
                _ => "Very weak",
            }
        }
    }

    /// "Nate's Home · Strong", "Wired · 192.168.1.20", "Not connected".
    pub fn summary(&self) -> String {
        if self.wired {
            format!("Wired{}", if self.ip.is_empty() { String::new() } else { format!(" \u{b7} {}", self.ip) })
        } else if self.wifi {
            let name = if self.ssid.is_empty() { "Wi-Fi" } else { self.ssid.as_str() };
            let s = self.strength();
            if s.is_empty() { name.to_string() } else { format!("{name} \u{b7} {s}") }
        } else {
            "Not connected".into()
        }
    }

    pub fn parse(&mut self, doc: &Value) -> bool {
        if doc.get("returnValue").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        let before = self.clone();
        let empty = json!({});
        let w = doc.get("wifi").unwrap_or(&empty);
        let e = doc.get("wired").unwrap_or(&empty);
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let int = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok())));
        self.wifi = text(w, "state") == "connected";
        self.wired = text(e, "state") == "connected";
        self.ssid = text(w, "ssid");
        let level = int(w, "signalLevel").or_else(|| int(w, "signalStrength"));
        self.dbm = level.map_or(String::new(), |l| (l as i64).to_string());
        self.bars = match int(w, "signalBars") {
            Some(b) => b.clamp(0.0, 4.0) as u8,
            None => match level {
                None => 0,
                Some(l) if l >= -55.0 => 4,
                Some(l) if l >= -65.0 => 3,
                Some(l) if l >= -75.0 => 2,
                Some(_) => 1,
            },
        };
        self.band = match int(w, "channelFrequency") {
            Some(f) if f > 0.0 => if f > 4000.0 { "5 GHz".into() } else { "2.4 GHz".into() },
            _ => text(w, "band"),
        };
        self.ip = text(if self.wired { e } else { w }, "ipAddress");
        self.method = text(if self.wired { e } else { w }, "method");
        *self != before
    }

    pub fn update(&mut self, input: &Input) -> bool {
        match input {
            Input::Luna { tag, payload } if *tag == TAG => self.parse(payload),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_is_the_link_the_name_and_the_strength() {
        let mut n = Net::new();
        assert_eq!(n.summary(), "Not connected");
        n.parse(&json!({"wifi": {"state": "connected", "ssid": "Nate's Home", "signalLevel": -50, "ipAddress": "192.168.1.9", "channelFrequency": 5180}}));
        assert_eq!(n.summary(), "Nate's Home \u{b7} Strong");
        assert_eq!((n.bars, n.band.as_str(), n.ip.as_str()), (4, "5 GHz", "192.168.1.9"));
        n.parse(&json!({"wifi": {"state": "connected", "ssid": "", "signalBars": 2}}));
        assert_eq!(n.summary(), "Wi-Fi \u{b7} Good");
        n.parse(&json!({"wifi": {"state": "connected", "signalLevel": -80}, "wired": {"state": "connected", "ipAddress": "192.168.1.20", "method": "dhcp"}}));
        assert_eq!(n.summary(), "Wired \u{b7} 192.168.1.20");
        assert_eq!(n.method, "dhcp");
        assert!(!n.parse(&json!({"returnValue": false})));
    }

    #[test]
    fn a_status_is_taken_only_from_its_own_subscription() {
        let mut n = Net::new();
        assert!(!n.update(&Input::Luna { tag: 1, payload: json!({"wifi": {"state": "connected"}}) }));
        assert!(n.update(&Input::Luna { tag: TAG, payload: json!({"wifi": {"state": "connected", "ssid": "A"}}) }));
        assert!(n.wifi);
    }
}
