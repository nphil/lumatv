//! Wi-Fi signal per wireless interface from `/proc/net/wireless`. The Go agent asks nl80211 for the
//! associated network's name as well; this reads the signal only.

/// Signal in dBm per interface; an interface without a plausible signal is not connected.
pub fn parse(text: &str) -> Vec<(String, f64)> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, data) = line.split_once(':')?;
            let level: f64 = data.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
            (-150.0..0.0).contains(&level).then(|| (name.trim().to_owned(), level))
        })
        .collect()
}

pub fn collect() -> Vec<(String, f64)> {
    std::fs::read_to_string("/proc/net/wireless").map(|text| parse(&text)).unwrap_or_default()
}

/// The Go agent's `Signals`: whole dBm, floored at the smallest `i8`.
pub fn signals(snapshot: &[(String, f64)]) -> Vec<(String, i8)> {
    snapshot.iter().map(|(name, signal)| (name.clone(), signal.round().max(f64::from(i8::MIN)) as i8)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_signal_of_connected_interfaces() {
        let text = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n \
                    face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n\
                    wlan0: 0000   70.  -40.6  -256        0      0      0      0      0        0\n\
                    wlan1: 0000    0.    0.     0        0      0      0      0      0        0\n";
        let snapshot = parse(text);
        assert_eq!(snapshot, [("wlan0".to_owned(), -40.6)]);
        assert_eq!(signals(&snapshot), [("wlan0".to_owned(), -41)]);
        assert_eq!(signals(&[("wlan0".to_owned(), -140.0)]), [("wlan0".to_owned(), -128)]);
    }
}
