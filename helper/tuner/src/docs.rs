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

/// An attribute value of the playlist: the quotes that would end it and the line breaks that would end the entry are out.
fn m3u_attr(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '"' { '\'' } else { c })
        .collect()
}

/// `/lineup.m3u`: the lineup as an IPTV playlist (VLC, Kodi, TiviMate, Jellyfin, Emby, Channels DVR). The channel's `tvg-id` is its
/// guide number, which is also its channel id in [`xmltv`], so a player joins the two by it.
pub fn m3u(entries: &[Entry], base: &str) -> String {
    let mut out = format!("#EXTM3U url-tvg=\"{base}/xmltv.xml\"\n");
    for e in entries {
        let (guide, name) = (m3u_attr(&e.guide), m3u_attr(&e.name));
        out.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"{guide}\" tvg-chno=\"{guide}\" tvg-name=\"{name}\",{guide} {name}\n{base}/auto/v{}\n",
            e.guide
        ));
    }
    out
}

/// Text for an XML element or attribute: escaped, and without the characters XML 1.0 cannot carry at all.
fn xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c < ' ' || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// An LG time string (`YYYY,MM,DD,hh,mm,ss`, UTC in `startTime` and `endTime`) as XMLTV writes it.
fn xmltv_time(text: &str) -> Option<String> {
    let mut f = text.trim().split(',').map(|p| p.trim().parse::<u32>().ok());
    let (y, mo, d, h, mi) = (f.next()??, f.next()??, f.next()??, f.next()??, f.next()??);
    let s = f.next().flatten().unwrap_or(0);
    if f.next().is_some()
        || y > 9999
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || s > 59
    {
        return None;
    }
    Some(format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02} +0000"))
}

fn text_of<'a>(p: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .filter_map(|k| p[*k].as_str())
        .map(str::trim)
        .find(|t| !t.is_empty())
        .unwrap_or("")
}

/// The title of the programme on `channel_id` at `now` (an LG time string) in its `iepg/getEventList` reply.
pub(crate) fn showing(reply: &Value, channel_id: &str, now: &str) -> Option<String> {
    let now = xmltv_time(now)?;
    reply["programList"]
        .as_array()?
        .iter()
        .filter(|p| p["channelId"].as_str().is_none_or(|id| id == channel_id))
        .find(|p| {
            let time = |key: &str| p[key].as_str().and_then(xmltv_time);
            time("startTime").is_some_and(|s| s <= now) && time("endTime").is_some_and(|e| now < e)
        })
        .map(|p| text_of(p, &["programName", "programMainTitle"]).to_owned())
        .filter(|t| !t.is_empty())
}

/// `/xmltv.xml`: the TV's own broadcast guide (`iepg/getEventList`, one reply per channel) as XMLTV. Channel ids are the guide numbers of
/// [`m3u`]; times are UTC. A programme without a title or a readable time is left out, a repeated start is kept once.
pub fn xmltv(channels: &[(Entry, Value)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tv generator-info-name=\"LumaTV\">\n",
    );
    for (e, _) in channels {
        out.push_str(&format!(
            "<channel id=\"{}\"><display-name>{}</display-name></channel>\n",
            xml_text(&e.guide),
            xml_text(&e.name)
        ));
    }
    for (e, reply) in channels {
        let mut list: Vec<(String, String, &Value)> = reply["programList"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["channelId"].as_str().is_none_or(|id| id == e.channel_id))
            .filter_map(|p| {
                let start = xmltv_time(p["startTime"].as_str()?)?;
                let stop = xmltv_time(p["endTime"].as_str()?)?;
                (stop > start && !text_of(p, &["programName", "programMainTitle"]).is_empty())
                    .then_some((start, stop, p))
            })
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list.dedup_by(|b, a| a.0 == b.0);
        for (start, stop, p) in list {
            out.push_str(&format!(
                "<programme start=\"{start}\" stop=\"{stop}\" channel=\"{}\"><title>{}</title>",
                xml_text(&e.guide),
                xml_text(text_of(p, &["programName", "programMainTitle"]))
            ));
            let sub = text_of(p, &["programSubTitle", "secondaryTitle"]);
            if !sub.is_empty() {
                out.push_str(&format!("<sub-title>{}</sub-title>", xml_text(sub)));
            }
            let desc = text_of(p, &["description"]);
            if !desc.is_empty() {
                out.push_str(&format!("<desc>{}</desc>", xml_text(desc)));
            }
            out.push_str("</programme>\n");
        }
    }
    out.push_str("</tv>\n");
    out
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

    fn entry(guide: &str, name: &str, id: &str) -> Entry {
        Entry {
            guide: guide.into(),
            name: name.into(),
            channel_id: id.into(),
            major: 0,
            minor: 0,
        }
    }

    #[test]
    fn the_playlist_has_one_entry_per_channel_and_names_the_guide() {
        let got = m3u(
            &[
                entry("2.1", "WSB-HD", "1_32_2_1_0_0_0"),
                entry("2.4", "WE \"ST\"\n,", "1_32_2_4_0_0_0"),
            ],
            "http://tv:5004",
        );
        assert_eq!(
            got,
            "#EXTM3U url-tvg=\"http://tv:5004/xmltv.xml\"\n\
             #EXTINF:-1 tvg-id=\"2.1\" tvg-chno=\"2.1\" tvg-name=\"WSB-HD\",2.1 WSB-HD\n\
             http://tv:5004/auto/v2.1\n\
             #EXTINF:-1 tvg-id=\"2.4\" tvg-chno=\"2.4\" tvg-name=\"WE 'ST',\",2.4 WE 'ST',\n\
             http://tv:5004/auto/v2.4\n"
        );
    }

    #[test]
    fn the_xmltv_guide_is_utc_escaped_and_keyed_by_guide_number() {
        let wsb = entry("2.1", "WSB & Co", "1_32_2_1_0_0_0");
        let west = entry("2.4", "WEST", "1_32_2_4_0_0_0");
        let p = |id: &str, name: &str, start: &str, end: &str| json!({"channelId": id, "programName": name, "startTime": start, "endTime": end});
        let list = json!({"programList": [
            {"channelId": "1_32_2_1_0_0_0", "programName": "News <live>", "programSubTitle": "Late", "description": "a & b\u{1}",
             "startTime": "2026,11,05,23,30,00", "endTime": "2026,11,06,00,00,00"},
            p("1_32_2_1_0_0_0", "Earlier", "2026,11,05,23,00,00", "2026,11,05,23,30,00"),
            p("1_32_2_1_0_0_0", "Twice", "2026,11,05,23,00,00", "2026,11,05,23,10,00"),
            p("1_32_2_1_0_0_0", "", "2026,11,06,00,00,00", "2026,11,06,01,00,00"),
            p("1_32_2_1_0_0_0", "Backwards", "2026,11,06,02,00,00", "2026,11,06,01,00,00"),
            p("1_32_2_1_0_0_0", "No time", "soon", "later"),
            p("1_32_9_9_0_0_0", "Another channel's", "2026,11,06,03,00,00", "2026,11,06,04,00,00"),
        ]});
        let got = xmltv(&[
            (wsb, list),
            (west, json!({"returnValue": true, "programList": []})),
        ]);
        assert_eq!(
            got,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tv generator-info-name=\"LumaTV\">\n\
             <channel id=\"2.1\"><display-name>WSB &amp; Co</display-name></channel>\n\
             <channel id=\"2.4\"><display-name>WEST</display-name></channel>\n\
             <programme start=\"20261105230000 +0000\" stop=\"20261105233000 +0000\" channel=\"2.1\"><title>Earlier</title></programme>\n\
             <programme start=\"20261105233000 +0000\" stop=\"20261106000000 +0000\" channel=\"2.1\"><title>News &lt;live&gt;</title><sub-title>Late</sub-title><desc>a &amp; b</desc></programme>\n\
             </tv>\n",
            "sorted by start; the repeated start, the untitled, backwards, unreadable and other channel's programmes are out"
        );
    }

    #[test]
    fn the_programme_showing_is_the_one_whose_hour_holds_now() {
        let p = |id: &str, name: &str, start: &str, end: &str| json!({"channelId": id, "programName": name, "startTime": start, "endTime": end});
        let list = json!({"programList": [
            p("1_32_2_1_0_0_0", "Earlier", "2026,11,05,23,00,00", "2026,11,05,23,30,00"),
            p("1_32_9_9_0_0_0", "Elsewhere", "2026,11,05,23,30,00", "2026,11,06,00,00,00"),
            p("1_32_2_1_0_0_0", "News", "2026,11,05,23,30,00", "2026,11,06,00,00,00"),
        ]});
        let at = |now: &str| showing(&list, "1_32_2_1_0_0_0", now);
        assert_eq!(at("2026,11,05,23,30,00").as_deref(), Some("News"));
        assert_eq!(at("2026,11,06,00,00,00"), None, "ended");
    }
}
