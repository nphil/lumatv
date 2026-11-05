//! The HDHomeRun documents: what Plex (and Jellyfin, Emby, Tvheadend) read to find a network tuner, see how many streams it can make and
//! list its channels. Pure functions of the TV's channel list and a name.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// One antenna channel of the lineup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `2.4`: what Plex shows and asks for (`/auto/v2.4`). Unique in a lineup, which Plex insists on.
    pub guide: String,
    pub name: String,
    /// LG's channel id, `1_<rf>_<major>_<minor>_0_0_0`: what the tuner is told to change to.
    pub channel_id: String,
    pub major: u16,
    pub minor: u16,
}

/// LG's id of an antenna channel: `1_32_2_4_0_0_0` is RF channel 32, virtual channel 2.4. Anything else (cable, analog, an input) is not
/// this tuner's.
pub fn parse_channel_id(id: &str) -> Option<(u8, u16, u16)> {
    let mut p = id.split('_');
    if p.next()? != "1" {
        return None;
    }
    let rf: u8 = p.next()?.parse().ok()?;
    let major: u16 = p.next()?.parse().ok()?;
    let minor: u16 = p.next()?.parse().ok()?;
    (rf >= 2 && major > 0).then_some((rf, major, minor))
}

/// The lineup of `iepg/getChannelList`'s reply: the antenna channels the owner has not skipped or hidden, one per number (a second
/// channel with the same number is dropped: a lineup lists each `GuideNumber` once), in number order.
pub fn entries(reply: &Value) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for c in reply["channelList"].as_array().into_iter().flatten() {
        let Some(id) = c["channelId"].as_str() else {
            continue;
        };
        let Some((_, major, minor)) = parse_channel_id(id) else {
            continue;
        };
        if c["skipped"].as_bool() == Some(true) || c["Invisible"].as_bool() == Some(true) {
            continue;
        }
        let guide = format!("{major}.{minor}");
        if out.iter().any(|e| e.guide == guide) {
            continue;
        }
        let name = c["channelName"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&guide)
            .to_owned();
        out.push(Entry {
            guide,
            name,
            channel_id: id.to_owned(),
            major,
            minor,
        });
    }
    out.sort_by_key(|e| (e.major, e.minor));
    out
}

/// What the tuner calls itself and how it is told from another: the name is the owner's, the id is the TV's (kept by Plex, so it must not
/// change from one start to the next).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub device_id: String,
}

/// Eight hex digits from the TV's own address, the same on every start.
pub fn device_id(mac: &str) -> String {
    let digest = Sha256::digest(format!("lumatv-hdhr:{}", mac.trim().to_ascii_lowercase()));
    digest[..4].iter().map(|b| format!("{b:02X}")).collect()
}

/// `/discover.json`. One tuner: a second stream is refused, whatever its channel.
pub fn discover(id: &Identity, base: &str) -> Value {
    json!({
        "FriendlyName": id.name,
        "Manufacturer": "Silicondust",
        "ModelNumber": "HDTC-2US",
        "FirmwareName": "hdhomeruntc_atsc",
        "FirmwareVersion": "20200101",
        "DeviceID": id.device_id,
        "DeviceAuth": "lumatv",
        "TunerCount": 1,
        "BaseURL": base,
        "LineupURL": format!("{base}/lineup.json"),
    })
}

/// `/lineup_status.json`: an antenna, never scanning (the TV's own channel list is the lineup, and scanning is the owner's, on the TV).
pub fn lineup_status() -> Value {
    json!({"ScanInProgress": 0, "ScanPossible": 1, "Source": "Antenna", "SourceList": ["Antenna"]})
}

/// `/lineup.json`.
pub fn lineup(entries: &[Entry], base: &str) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|e| json!({"GuideNumber": e.guide, "GuideName": e.name, "URL": format!("{base}/auto/v{}", e.guide)}))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_antenna_channel_id_names_its_rf_channel_and_number() {
        assert_eq!(parse_channel_id("1_32_2_4_0_0_0"), Some((32, 2, 4)));
        assert_eq!(parse_channel_id("1_27_5_1_0_0_0"), Some((27, 5, 1)));
        for not in [
            "2_32_2_4_0_0_0",
            "1_1_2_4",
            "1_32_0_1",
            "1_32_x_1",
            "",
            "HDMI_1",
        ] {
            assert_eq!(parse_channel_id(not), None, "{not}");
        }
    }

    fn reply() -> Value {
        json!({"channelList": [
            {"channelId": "1_32_2_4_0_0_0", "channelName": "WEST", "skipped": false, "Invisible": false},
            {"channelId": "1_19_17_1_0_0_0", "channelName": " WPCH-TV ", "skipped": false},
            {"channelId": "1_32_2_1_0_0_0", "channelName": "WSB-HD"},
            {"channelId": "1_29_16_1_0_0_0", "channelName": "WYGA-CD"},
            {"channelId": "1_29_16_2_0_0_0", "channelName": "WYGA-CD"},
            {"channelId": "1_21_16_1_0_0_0", "channelName": "a second 16.1"},
            {"channelId": "1_35_35_2_0_0_0", "channelName": "ESPANOL", "skipped": true},
            {"channelId": "1_36_69_1_0_0_0", "channelName": "WUPA-HD", "Invisible": true},
            {"channelId": "2_1_9_9_0_0_0", "channelName": "a cable channel"},
            {"channelName": "no id"},
            {"channelId": "1_18_34_1_0_0_0", "channelName": ""}
        ]})
    }

    #[test]
    fn the_lineup_is_the_antenna_channels_once_each_in_number_order() {
        let got = entries(&reply());
        assert_eq!(
            got.iter()
                .map(|e| (e.guide.as_str(), e.name.as_str(), e.channel_id.as_str()))
                .collect::<Vec<_>>(),
            [
                ("2.1", "WSB-HD", "1_32_2_1_0_0_0"),
                ("2.4", "WEST", "1_32_2_4_0_0_0"),
                ("16.1", "WYGA-CD", "1_29_16_1_0_0_0"),
                ("16.2", "WYGA-CD", "1_29_16_2_0_0_0"),
                ("17.1", "WPCH-TV", "1_19_17_1_0_0_0"),
                ("34.1", "34.1", "1_18_34_1_0_0_0"),
            ],
            "skipped, hidden, non-antenna and repeated numbers are out; a trimmed name; no name is the number"
        );
    }

    #[test]
    fn the_documents_say_one_tuner_an_antenna_and_where_each_stream_is() {
        let id = Identity {
            name: "LumaTV".into(),
            device_id: device_id("AA:BB:CC:00:11:22"),
        };
        let base = "http://192.168.1.242:5004";
        let d = discover(&id, base);
        assert_eq!(
            (
                d["TunerCount"].clone(),
                d["BaseURL"].as_str(),
                d["LineupURL"].as_str()
            ),
            (
                json!(1),
                Some(base),
                Some("http://192.168.1.242:5004/lineup.json")
            )
        );
        assert_eq!(
            (d["FriendlyName"].as_str(), d["ModelNumber"].as_str()),
            (Some("LumaTV"), Some("HDTC-2US"))
        );
        assert_eq!(lineup_status()["Source"], "Antenna");
        assert_eq!(lineup_status()["ScanInProgress"], 0);
        let l = lineup(&entries(&reply()), base);
        assert_eq!(
            l[1],
            json!({"GuideNumber": "2.4", "GuideName": "WEST", "URL": "http://192.168.1.242:5004/auto/v2.4"})
        );
    }

    #[test]
    fn the_device_id_is_eight_hex_digits_and_the_same_for_the_same_address() {
        let id = device_id("aa:bb:cc:00:11:22");
        assert_eq!(id.len(), 8);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase())
        );
        assert_eq!(id, device_id(" AA:BB:CC:00:11:22\n"));
        assert_ne!(id, device_id("aa:bb:cc:00:11:23"));
    }
}
