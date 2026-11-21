//! The TV's uplink: the port the default route leaves by, whether it is wired or Wi-Fi, and how fast a wired one runs.
//!
//! A port that negotiated 100 Mbit/s cannot carry what the stream settings go up to, and a stream that fills the port leaves no room for
//! packet headers, audio, retransmissions or a burst, so the bitrate each engine is asked for stops at [`SHARE_PERCENT`] of a wired port's
//! speed. Wi-Fi is only reported: the rate its driver gives is the radio's best case, not what it delivers, so it limits nothing.
//!
//! The app's jail mounts `/sys/class/net` and `/sys/devices` read-only (`jail_native_devmode.conf`), so the app reads the uplink itself.
use std::path::Path;

/// The part of a wired port's speed a stream may ask for; the rest is headers, audio, control traffic and headroom.
pub const SHARE_PERCENT: u32 = 85;
/// The Streaming bitrate slider's range and step in Mb/s. An engine's schema refuses a value off its step.
pub const BITRATE_MIN_MBPS: u32 = 5;
pub const BITRATE_MAX_MBPS: u32 = 150;
pub const BITRATE_STEP_MBPS: u32 = 5;
/// How often the shell rereads the uplink.
const EVERY_MS: u64 = 3_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Wired,
    Wifi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uplink {
    pub interface: String,
    pub medium: Medium,
    /// What the port negotiated, in Mbit/s. Absent when the driver does not say, which is also how a port without a link reads.
    pub mbit: Option<u32>,
}

impl Uplink {
    /// The most a stream may ask for, in Mb/s on the slider's steps. Absent when this uplink limits nothing: Wi-Fi, or a port whose
    /// speed is unknown.
    pub fn cap_mbps(&self) -> Option<u32> {
        let mbit = self.mbit.filter(|_| self.medium == Medium::Wired)?;
        let share = mbit.saturating_mul(SHARE_PERCENT) / 100;
        Some((share / BITRATE_STEP_MBPS * BITRATE_STEP_MBPS).max(BITRATE_STEP_MBPS))
    }

    /// What the engines are asked for when the viewer chose `wanted` Mb/s.
    pub fn limit(&self, wanted: u32) -> u32 {
        self.cap_mbps().map_or(wanted, |cap| wanted.min(cap))
    }

    /// The line under the Bitrate setting. Absent when the uplink has nothing to say: the port could not limit a bitrate the slider reaches.
    pub fn note(&self, wanted: u32) -> Option<String> {
        match (self.medium, self.cap_mbps(), self.mbit) {
            (Medium::Wifi, ..) => Some("The TV is on Wi-Fi, so no limit is applied".into()),
            (Medium::Wired, Some(cap), Some(mbit)) if wanted > cap => Some(format!("Limited to {cap} Mb/s by the TV's {mbit} Mb/s port")),
            (Medium::Wired, Some(cap), Some(mbit)) if cap < BITRATE_MAX_MBPS => Some(format!("The TV's {mbit} Mb/s port allows up to {cap} Mb/s")),
            _ => None,
        }
    }
}

/// The interface the default route leaves by: of the routes that are up and match every destination, the one with the lowest metric.
/// `routes` is `/proc/net/route`.
fn default_interface(routes: &str) -> Option<&str> {
    const UP: u32 = 0x1;
    const REJECT: u32 = 0x200;
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut field = line.split_whitespace();
            let interface = field.next()?;
            let (destination, flags, metric, mask) = (field.next()?, field.nth(1)?, field.nth(2)?, field.next()?);
            let flags = u32::from_str_radix(flags, 16).ok()?;
            let default = destination == "00000000" && mask == "00000000" && flags & UP != 0 && flags & REJECT == 0;
            default.then_some((metric.parse::<u32>().ok()?, interface))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, interface)| interface)
}

/// The uplink the default route in `routes` leaves by. `read` gives an interface's sysfs attribute and `exists` says whether one is there,
/// so a Wi-Fi radio (it has a `wireless` directory, and a `phy80211` link the jail cannot follow) is told from a wire without opening either.
pub fn detect(routes: &str, read: impl Fn(&str, &str) -> Option<String>, exists: impl Fn(&str, &str) -> bool) -> Option<Uplink> {
    let interface = default_interface(routes)?;
    let wifi = exists(interface, "wireless") || exists(interface, "phy80211");
    // A port without a link, and a driver without ethtool, read as -1 or fail with EINVAL.
    let mbit = read(interface, "speed").and_then(|s| s.trim().parse::<u32>().ok()).filter(|mbit| *mbit > 0);
    Some(Uplink { interface: interface.into(), medium: if wifi { Medium::Wifi } else { Medium::Wired }, mbit })
}

/// The TV's uplink now; absent when it has no default route.
pub fn read() -> Option<Uplink> {
    const NET: &str = "/sys/class/net";
    let routes = std::fs::read_to_string("/proc/net/route").ok()?;
    detect(
        &routes,
        |interface, attribute| std::fs::read_to_string(Path::new(NET).join(interface).join(attribute)).ok(),
        |interface, attribute| Path::new(NET).join(interface).join(attribute).symlink_metadata().is_ok(),
    )
}

/// Rereads the uplink every few seconds and reports it when it changed, and the first time.
#[derive(Default)]
pub struct Watch {
    next_ms: u64,
    last: Option<Option<Uplink>>,
}
impl Watch {
    pub fn poll(&mut self, now_ms: u64, read: impl FnOnce() -> Option<Uplink>) -> Option<Option<Uplink>> {
        if self.last.is_some() && now_ms < self.next_ms {
            return None;
        }
        self.next_ms = now_ms.saturating_add(EVERY_MS);
        let uplink = read();
        if self.last.as_ref() == Some(&uplink) {
            return None;
        }
        self.last = Some(uplink.clone());
        Some(uplink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT";
    /// The TV's table, read over SSH: one wired default route.
    const TV: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
        eth0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
        eth0\t9CD1BD20\t0101A8C0\t0007\t0\t0\t0\tFFFFFFFF\t0\t0\t0\n\
        eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n";

    fn wired(mbit: Option<u32>) -> Uplink {
        Uplink { interface: "eth0".into(), medium: Medium::Wired, mbit }
    }
    fn wifi(mbit: Option<u32>) -> Uplink {
        Uplink { interface: "wlan0".into(), medium: Medium::Wifi, mbit }
    }

    #[test]
    fn a_hundred_megabit_port_caps_a_stream_at_85() {
        assert_eq!(wired(Some(100)).cap_mbps(), Some(85));
        assert_eq!(wired(Some(100)).limit(150), 85);
        assert_eq!(wired(Some(100)).limit(80), 80);
    }

    #[test]
    fn the_cap_is_a_whole_number_of_slider_steps_and_never_below_the_slider() {
        for (mbit, cap) in [(10, 5), (25, 20), (100, 85), (1_000, 850), (2_500, 2_125), (10_000, 8_500)] {
            assert_eq!(wired(Some(mbit)).cap_mbps(), Some(cap), "{mbit}");
            assert!(cap % BITRATE_STEP_MBPS == 0 && cap >= BITRATE_MIN_MBPS, "{mbit}");
        }
    }

    #[test]
    fn a_gigabit_port_limits_nothing_the_slider_reaches() {
        assert_eq!(wired(Some(1_000)).limit(BITRATE_MAX_MBPS), BITRATE_MAX_MBPS);
    }

    #[test]
    fn wifi_and_an_unknown_speed_limit_nothing() {
        assert_eq!((wifi(Some(54)).cap_mbps(), wifi(Some(54)).limit(150)), (None, 150));
        assert_eq!((wired(None).cap_mbps(), wired(None).limit(150)), (None, 150));
    }

    #[test]
    fn the_note_says_what_limits_the_bitrate_and_stays_quiet_when_nothing_does() {
        assert_eq!(wired(Some(100)).note(150).as_deref(), Some("Limited to 85 Mb/s by the TV's 100 Mb/s port"));
        assert_eq!(wired(Some(100)).note(85).as_deref(), Some("The TV's 100 Mb/s port allows up to 85 Mb/s"));
        assert_eq!(wired(Some(100)).note(80).as_deref(), Some("The TV's 100 Mb/s port allows up to 85 Mb/s"));
        assert_eq!(wired(Some(1_000)).note(150), None);
        assert_eq!(wired(None).note(150), None);
        assert_eq!(wifi(None).note(150).as_deref(), Some("The TV is on Wi-Fi, so no limit is applied"));
    }

    #[test]
    fn the_default_route_with_the_lowest_metric_names_the_interface() {
        assert_eq!(default_interface(TV), Some("eth0"));
        let two = format!("{HEADER}\nwlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\neth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n");
        assert_eq!(default_interface(&two), Some("eth0"));
    }

    #[test]
    fn a_route_that_is_down_or_rejects_or_is_not_the_default_names_nothing() {
        let routes = format!(
            "{HEADER}\neth0\t00000000\t0101A8C0\t0002\t0\t0\t0\t00000000\t0\t0\t0\neth0\t00000000\t00000000\t0201\t0\t0\t0\t00000000\t0\t0\t0\n\
             eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\nshort line\n"
        );
        assert_eq!(default_interface(&routes), None);
        assert_eq!(default_interface(""), None);
    }

    /// Fake sysfs: `(interface, attribute, contents)`. An empty content is an attribute that is there and cannot be read.
    fn sysfs<'a>(files: &'a [(&'a str, &'a str, &'a str)]) -> (impl Fn(&str, &str) -> Option<String> + 'a, impl Fn(&str, &str) -> bool + 'a) {
        (
            move |interface: &str, attribute: &str| files.iter().find(|f| f.0 == interface && f.1 == attribute && !f.2.is_empty()).map(|f| f.2.to_string()),
            move |interface: &str, attribute: &str| files.iter().any(|f| f.0 == interface && f.1 == attribute),
        )
    }

    #[test]
    fn the_tv_reads_as_a_wired_hundred_megabit_uplink() {
        let (read, exists) = sysfs(&[("eth0", "speed", "100\n"), ("eth0", "duplex", "full\n"), ("wlan0", "wireless", "")]);
        assert_eq!(detect(TV, read, exists), Some(wired(Some(100))));
    }

    #[test]
    fn a_radio_reads_as_wifi_by_either_of_its_marks_and_a_dead_speed_as_unknown() {
        for mark in ["wireless", "phy80211"] {
            let files = [("wlan0", mark, ""), ("wlan0", "speed", "-1\n")];
            let (read, exists) = sysfs(&files);
            assert_eq!(detect(&TV.replace("eth0", "wlan0"), read, exists), Some(wifi(None)), "{mark}");
        }
        let (read, exists) = sysfs(&[("eth0", "speed", "")]);
        assert_eq!(detect(TV, read, exists), Some(wired(None)));
    }

    #[test]
    fn no_default_route_is_no_uplink() {
        let (read, exists) = sysfs(&[]);
        assert_eq!(detect(HEADER, read, exists), None);
    }

    #[test]
    fn the_watch_reads_when_due_and_reports_only_a_change() {
        let mut watch = Watch::default();
        let mut reads = 0;
        let mut poll = |watch: &mut Watch, now: u64, uplink: Option<Uplink>| {
            watch.poll(now, || {
                reads += 1;
                uplink
            })
        };
        assert_eq!(poll(&mut watch, 0, None), Some(None), "the first reading is always reported");
        assert_eq!(poll(&mut watch, 1_000, Some(wired(Some(100)))), None, "not due yet");
        assert_eq!(poll(&mut watch, 3_000, Some(wired(Some(100)))), Some(Some(wired(Some(100)))));
        assert_eq!(poll(&mut watch, 6_000, Some(wired(Some(100)))), None, "unchanged");
        assert_eq!(poll(&mut watch, 9_000, Some(wired(Some(1_000)))), Some(Some(wired(Some(1_000)))));
        assert_eq!(poll(&mut watch, 12_000, None), Some(None), "the uplink went away");
        assert_eq!(reads, 5);
    }
}
