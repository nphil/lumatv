//! LG's channels and programmes, as the services behind LG's own Live TV banner and guide give them: the guide service `iepg` (the channel
//! list, the programmes of a channel for a window of time) and the broadcast service `utp` (the channel the tuner is on, the last one
//! tuned). Pure: the requests are built here, the answers parsed here, and nothing is sent or drawn; the models ask through
//! [`Effect::Luna`](crate::kit::screen::Effect::Luna) and read the answers they get as [`Input::Luna`](crate::kit::screen::Input::Luna).
//! The Live TV channel banner and the guide share this one layer.
//!
//! Every shape below was read from the set (read-only `luna-send`, 2026-11-03, a US antenna set with LG Channels):
//!
//! - `iepg/getChannelList` with LG's own query (`dataType 0`, `sort 0`, `channelGroup ["All"]`, `channelMode ["Tuner"]` and/or `["IP"]`, a
//!   `requestField` list) answers `{returnValue, channelListCount, channelList: [{channelId "1_32_2_1_0_0_0", channelNumber "2-1",
//!   channelName "WSB-HD", imgUrl "/mnt/lg/cmn_data/channel_logo/major_logo_img/19586_..._h356x200.png", ...}], tuner_channel, ip_channel}`.
//!   57 antenna channels (`channelMode "Terrestrial"`, `channelTypeId` 1) and 459 LG Channels (`channelMode "IP"`, `channelTypeId` 20,
//!   ids `11_3_<number>_65535_0_0_0`); `lastUpdated` and `programId` come whatever is asked. The antenna list with the fields below is 31 KB, the
//!   internet list 150 KB. The logo is a local PNG (356 by 200) of the set's own file system, or empty; the UI process reads it.
//! - `iepg/getEventList {dataType 0, localStartTime, localEndTime, channelId [..]}` answers `{returnValue, programListCount, programList:
//!   [{programId, channelId, programName, description, duration (s), startTime/endTime (UTC), localStartTime/localEndTime (the
//!   broadcast's wall clock), rating [{dimension [{dim, dimRatingString}]}], isSubtitle, ...}]}`; the times are `YYYY,MM,DD,hh,mm,ss`. A
//!   programme that overlaps the start of the window is in it. A programme is about 3.7 KB (`requestField` is not honoured), so a model asks
//!   for few channels and few hours at a time. A channel the set has no guide data for answers no programmes (the LG Channels all do).
//! - `iepg/getCurrentEventList` and `getCurrentNextEventList {dataType 0, channelId [..]}` answer the same `programList` (now, and now and
//!   what follows).
//! - `utp.channel/getLastChannelId {sourceType: "INPUTSOURCE_RF"}` answers `{returnValue, channelId, mode: "INPUTSOURCE_TERRESTRIAL_DIGITAL"}`, the
//!   channel the tuner is on or was last on, with no pipeline (`subscribe` is LG's own banner's way to follow it);
//!   `utp.broadcast/getCurrentChannel {broadcastId}` answers `{returnValue, channel: {channelId, channelNumber, channelName, channelTypeId, ...}}`
//!   and needs the id of a live pipeline: with an empty `broadcastId` it answers `errorCode -106` ("No resource") while Live TV is not running.
//! - `iepg/getChannelInfo {dataType 0, channelId}` answers `{returnValue, channelList: [{channelId, channelNumber, channelName, imgUrl, ...}]}`,
//!   one channel with every field.
//!
//! Not read on the set (it changes the channel): the tune call. LG's guide hands the channel to Live TV by launching it
//! (`inputcommon`'s `DeepLinkHandler.qml`: `{id: "channelChange", channelId, sender: "livemenu", type}`), which is [`tune_request`]; the
//! type `tvguide` lets LG's own channel banner show after the change, any other (`channels`) blocks it, so that a banner of LumaTV's is alone.

use crate::host::clock;
use serde_json::{Value, json};

pub const LIST_URI: &str = "luna://com.webos.service.iepg/getChannelList";
pub const EVENTS_URI: &str = "luna://com.webos.service.iepg/getEventList";
pub const NOW_URI: &str = "luna://com.webos.service.iepg/getCurrentEventList";
pub const NOW_NEXT_URI: &str = "luna://com.webos.service.iepg/getCurrentNextEventList";
pub const EVENTS_CHANGED_URI: &str = "luna://com.webos.service.iepg/notifyEventChanged";
pub const LIST_CHANGED_URI: &str = "luna://com.webos.service.iepg/notifyChannelListChanged";
pub const LAST_URI: &str = "luna://com.webos.service.utp.channel/getLastChannelId";
pub const INFO_URI: &str = "luna://com.webos.service.iepg/getChannelInfo";
pub const CURRENT_URI: &str = "luna://com.webos.service.utp.broadcast/getCurrentChannel";
pub const LAUNCH_URI: &str = "luna://com.webos.applicationManager/launch";
/// LG's Live TV, which tunes.
pub const LIVE_TV_APP: &str = "com.webos.app.livetv";

/// The tags of the calls this layer's users make, as a base: a model adds its own offsets.
pub const TAG_BASE: u64 = 0x4743_0000;

/// What a request to a luna service that is not answering or not allowed looks like: `{returnValue: false, errorText, errorCode}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The service said no (its `errorText`, its `errorCode` when it gave one).
    Refused { text: String, code: i64 },
    /// An answer that is not the shape this layer knows.
    Shape(&'static str),
}

impl Fault {
    /// A few words for the person: what failed, without the service's own jargon.
    pub fn words(&self) -> &'static str {
        match self {
            Fault::Refused { .. } => "The guide service did not answer",
            Fault::Shape(_) => "The guide gave something unexpected",
        }
    }
}

/// Where a channel comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// The tuner: antenna, cable or satellite.
    Tuner,
    /// LG Channels and other internet channels (`channelMode` `IP`).
    Internet,
}

impl Source {
    /// The word LG's own `channelMode` filter takes.
    pub fn mode(self) -> &'static str {
        match self {
            Source::Tuner => "Tuner",
            Source::Internet => "IP",
        }
    }
}

/// One channel of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// LG's id (`1_32_2_1_0_0_0`): what a request about the channel names and what tunes it.
    pub id: String,
    /// As LG writes it: `2-1` on the antenna, `100` for an internet channel.
    pub number: String,
    pub name: String,
    /// The logo's file on the set, when it has one.
    pub logo: Option<String>,
    pub source: Source,
    /// LG's `channelMode` word for the channel (`Terrestrial`, `Cable`, `Satellite`, `IP`); empty when the request did not ask for it.
    pub mode: String,
    /// The favourite groups A to D the channel is in, one bit each (A is bit 0).
    pub favourites: u8,
}

impl Channel {
    /// Whether the channel is in any favourite group.
    pub fn favourite(&self) -> bool {
        self.favourites != 0
    }

    /// `(major, minor)` of the number for ordering: `2-1` is (2, 1), `100` is (100, 0), what is not a number sorts last.
    pub fn number_key(&self) -> (u32, u32) {
        let mut parts = self.number.split(['-', '.']);
        let major = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(u32::MAX);
        let minor = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
        (major, minor)
    }
}

/// One programme of one channel, in epoch milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Programme {
    pub id: String,
    pub channel: String,
    pub title: String,
    /// The episode's own title, when the broadcast gives one.
    pub subtitle: String,
    pub text: String,
    pub start: i64,
    pub end: i64,
    /// `TV-14 DL`: the rating and its descriptors, empty when the broadcast gives none.
    pub rating: String,
    pub captions: bool,
    pub described: bool,
}

impl Programme {
    pub fn minutes(&self) -> i64 {
        (self.end - self.start) / 60_000
    }

    /// Whether the programme is on the air at `now`.
    pub fn airing(&self, now_ms: i64) -> bool {
        self.start <= now_ms && now_ms < self.end
    }

    /// How much of the programme has been broadcast at `now`, 0 to 1.
    pub fn progress(&self, now_ms: i64) -> f32 {
        let span = (self.end - self.start).max(1) as f32;
        ((now_ms - self.start) as f32 / span).clamp(0.0, 1.0)
    }
}

/// The programme on the air at `now` among `list` (sorted by start), and the one after it.
pub fn now_next(list: &[Programme], now_ms: i64) -> (Option<&Programme>, Option<&Programme>) {
    let i = list.partition_point(|p| p.end <= now_ms);
    match list.get(i) {
        Some(p) if p.start <= now_ms => (Some(p), list.get(i + 1)),
        other => (None, other),
    }
}

/// The channel the tuner is on, as `getCurrentChannel` says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    pub id: String,
    pub number: String,
    pub name: String,
}

// ---------------------------------------------------------------------------------------------------------------------- requests

/// The `getChannelList` request of LG's own surface manager (`FavoriteChannels/AvailableChannelModel.qml`), for the channels of one source:
/// the tuner's with what a guide filters on (the flags, the favourite groups), the internet's with the least (459 of them are 150 KB).
pub fn list_request(source: Source) -> Value {
    let fields: &[&str] = match source {
        Source::Tuner => &["channelId", "channelNumber", "channelName", "channelMode", "imgUrl", "channelTypeId", "skipped", "Invisible", "favoriteIdxA", "favoriteIdxB", "favoriteIdxC", "favoriteIdxD"],
        Source::Internet => &["channelId", "channelNumber", "channelName", "imgUrl"],
    };
    json!({"dataType": 0, "sort": 0, "channelMode": [source.mode()], "channelGroup": ["All"], "requestField": fields})
}

/// `YYYY,MM,DD,hh,mm,ss` for the instant `epoch_ms` on a wall clock `offset_s` seconds east of UTC (LG's time strings).
pub fn stamp(epoch_ms: i64, offset_s: i32) -> String {
    let c = clock::civil_at(epoch_ms.max(0) as u64, offset_s);
    format!("{:04},{:02},{:02},{:02},{:02},{:02}", c.year, c.month, c.day, c.hour, c.minute, c.second)
}

/// The instant of an LG time string whose fields are on a clock `offset_s` seconds east of UTC (0 for the UTC fields).
pub fn parse_stamp(text: &str, offset_s: i32) -> Option<i64> {
    let mut it = text.trim().split(',').map(|p| p.trim().parse::<i32>().ok());
    let (y, mo, d, h, mi, s) = (it.next()??, it.next()??, it.next()??, it.next()??, it.next()??, it.next().flatten().unwrap_or(0));
    if it.next().is_some() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..60).contains(&s) {
        return None;
    }
    Some(clock::epoch_of(y, mo as u8, d as u8, h as u8, mi as u8, s as u8, offset_s) as i64)
}

/// The `getEventList` request for the programmes of `channels` between `from_ms` and `to_ms`, on the broadcast's wall clock `offset_s`.
pub fn events_request(channels: &[&str], from_ms: i64, to_ms: i64, offset_s: i32) -> Value {
    json!({"dataType": 0, "localStartTime": stamp(from_ms, offset_s), "localEndTime": stamp(to_ms, offset_s), "channelId": channels})
}

/// The `getCurrentNextEventList` request: what is on now and what follows, for `channels`.
pub fn now_next_request(channels: &[&str]) -> Value {
    json!({"dataType": 0, "channelId": channels})
}

/// The launch that tunes `channel_id` as LG's own guide does: Live TV opened (or told, when it is in front) with the channel to change to. The
/// type is `channels`, which keeps LG's own channel banner from showing over a banner of LumaTV's (`tvguide` would let it).
pub fn tune_request(channel_id: &str) -> Value {
    json!({"id": LIVE_TV_APP, "params": {"id": "channelChange", "channelId": channel_id, "sender": "livemenu", "type": "channels"}})
}

/// The `getCurrentChannel` request on a live pipeline (`broadcast_id`; empty answers -106 while Live TV is not running).
pub fn current_request(broadcast_id: &str, subscribe: bool) -> Value {
    json!({"broadcastId": broadcast_id, "subscribe": subscribe})
}

/// The `getLastChannelId` request: the channel the tuner is on or was last on (`subscribe` follows it).
pub fn last_request(subscribe: bool) -> Value {
    json!({"sourceType": "INPUTSOURCE_RF", "subscribe": subscribe})
}

/// The `getChannelInfo` request for one channel.
pub fn info_request(channel_id: &str) -> Value {
    json!({"dataType": 0, "channelId": channel_id})
}

// ---------------------------------------------------------------------------------------------------------------------- answers

fn checked(payload: &Value) -> Result<(), Fault> {
    if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return Err(Fault::Refused {
            text: payload.get("errorText").and_then(Value::as_str).unwrap_or("").to_string(),
            code: payload.get("errorCode").and_then(Value::as_i64).unwrap_or(0),
        });
    }
    Ok(())
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).map_or("", str::trim)
}

/// A logo that is a file of the set: a path (or `file://`), not a web address.
fn logo_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let path = raw.strip_prefix("file://").unwrap_or(raw);
    path.starts_with('/').then(|| path.to_string())
}

/// The channels of a `getChannelList` answer in the order the service gave, without the ones LG hides (skipped, invisible) or that name
/// nothing. `source` is the one the request was for.
pub fn parse_channels(payload: &Value, source: Source) -> Result<Vec<Channel>, Fault> {
    checked(payload)?;
    let list = payload.get("channelList").and_then(Value::as_array).ok_or(Fault::Shape("no channelList"))?;
    let mut out = Vec::with_capacity(list.len());
    for c in list {
        let id = text(c, "channelId");
        if id.is_empty() || c.get("skipped").and_then(Value::as_bool) == Some(true) || c.get("Invisible").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let number = text(c, "channelNumber");
        let name = text(c, "channelName");
        let mut favourites = 0u8;
        for (bit, key) in ["favoriteIdxA", "favoriteIdxB", "favoriteIdxC", "favoriteIdxD"].iter().enumerate() {
            // 250 and 255 are "not in the group"; a place in it is a small number
            if c.get(*key).and_then(Value::as_u64).is_some_and(|n| n < 250) {
                favourites |= 1 << bit;
            }
        }
        out.push(Channel {
            id: id.to_string(),
            number: number.to_string(),
            name: if name.is_empty() { number.to_string() } else { name.to_string() },
            logo: logo_path(text(c, "imgUrl")),
            source,
            mode: text(c, "channelMode").to_string(),
            favourites,
        });
    }
    Ok(out)
}

/// The two lists put in the order of the channel numbers (the antenna's `2-1` before the internet's `100`).
pub fn merge(mut channels: Vec<Channel>) -> Vec<Channel> {
    channels.sort_by_key(Channel::number_key);
    channels
}

/// The rating of a programme as the person reads it: `TV-14 DL` (the broadcast writes `TV TV-14` for the rating and one letter for each
/// descriptor).
fn rating_of(p: &Value) -> String {
    let Some(first) = p.get("rating").and_then(Value::as_array).and_then(|r| r.first()) else { return String::new() };
    let Some(dims) = first.get("dimension").and_then(Value::as_array) else { return String::new() };
    let mut head = String::new();
    let mut tail = String::new();
    for d in dims {
        let s = text(d, "dimRatingString");
        if s.is_empty() {
            continue;
        }
        if head.is_empty() && d.get("dim").and_then(Value::as_i64) == Some(0) {
            head = s.strip_prefix("TV ").unwrap_or(s).to_string();
        } else {
            tail.push_str(s);
        }
    }
    match (head.is_empty(), tail.is_empty()) {
        (true, true) => String::new(),
        (false, true) => head,
        (true, false) => tail,
        (false, false) => format!("{head} {tail}"),
    }
}

/// The wall clock's offset from UTC the broadcast uses, in seconds, read from the first programme that gives both its UTC and its local
/// start (a whole quarter hour), so that times asked for and shown agree with the broadcast's day.
pub fn offset_of(payload: &Value) -> Option<i32> {
    let list = payload.get("programList").and_then(Value::as_array)?;
    list.iter().find_map(|p| {
        let utc = parse_stamp(text(p, "startTime"), 0)?;
        let local = parse_stamp(text(p, "localStartTime"), 0)?;
        let secs = (local - utc) / 1000;
        Some((((secs as f64) / 900.0).round() as i32) * 900)
    })
}

/// The programmes of a `getEventList`, `getCurrentEventList` or `getCurrentNextEventList` answer, by start. The instant is the UTC time
/// the broadcast gives; one that gives only its local time is read on the clock `offset_s` seconds east of UTC.
pub fn parse_programmes(payload: &Value, offset_s: i32) -> Result<Vec<Programme>, Fault> {
    checked(payload)?;
    let list = payload.get("programList").and_then(Value::as_array).ok_or(Fault::Shape("no programList"))?;
    let mut out: Vec<Programme> = Vec::with_capacity(list.len());
    for p in list {
        let channel = match text(p, "channelId") {
            "" => text(p, "signalChannelId"),
            id => id,
        };
        let start = parse_stamp(text(p, "startTime"), 0).or_else(|| parse_stamp(text(p, "localStartTime"), offset_s));
        let end = parse_stamp(text(p, "endTime"), 0)
            .or_else(|| parse_stamp(text(p, "localEndTime"), offset_s))
            .or_else(|| start.zip(p.get("duration").and_then(Value::as_i64)).map(|(s, d)| s + d * 1000));
        let (Some(start), Some(end)) = (start, end) else { continue };
        let title = match text(p, "programName") {
            "" => text(p, "programMainTitle"),
            t => t,
        };
        if channel.is_empty() || end <= start {
            continue;
        }
        out.push(Programme {
            id: text(p, "programId").to_string(),
            channel: channel.to_string(),
            title: title.to_string(),
            subtitle: match text(p, "programSubTitle") {
                "" => text(p, "secondaryTitle").to_string(),
                t => t.to_string(),
            },
            text: text(p, "description").to_string(),
            start,
            end,
            rating: rating_of(p),
            captions: p.get("isSubtitle").and_then(Value::as_bool) == Some(true),
            described: p.get("isAD").and_then(Value::as_bool) == Some(true),
        });
    }
    out.sort_by(|a, b| (a.channel.as_str(), a.start).cmp(&(b.channel.as_str(), b.start)));
    out.dedup_by(|b, a| a.channel == b.channel && a.start == b.start);
    Ok(out)
}

/// The channel `utp.channel/getLastChannelId` names.
pub fn parse_last(payload: &Value) -> Option<String> {
    checked(payload).ok()?;
    Some(text(payload, "channelId")).filter(|id| !id.is_empty()).map(str::to_string)
}

/// The channel a `getChannelInfo` answer describes (an internet channel is told by its `11_` id).
pub fn parse_channel_info(payload: &Value) -> Option<Channel> {
    checked(payload).ok()?;
    let first = payload.get("channelList")?.as_array()?.first()?.clone();
    let source = if text(&first, "channelId").starts_with("11_") { Source::Internet } else { Source::Tuner };
    parse_channels(&json!({"returnValue": true, "channelList": [first]}), source).ok()?.into_iter().next()
}

/// The channel `utp.broadcast/getCurrentChannel` says the tuner is on.
pub fn parse_current(payload: &Value) -> Option<Current> {
    checked(payload).ok()?;
    let c = payload.get("channel")?;
    let id = text(c, "channelId");
    (!id.is_empty()).then(|| Current { id: id.to_string(), number: text(c, "channelNumber").to_string(), name: text(c, "channelName").to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from the set's answer (the antenna's first channels, an internet one and two LG hides).
    fn list() -> Value {
        json!({
            "returnValue": true, "channelListCount": 5, "dataType": 0,
            "tuner_channel": {"channelListCount": 57, "returnValue": true},
            "channelList": [
                {"channelId": "1_32_2_1_0_0_0", "channelNumber": "2-1", "channelName": "WSB-HD", "lastUpdated": "", "programId": "1_32_2_1_0_0_0",
                 "imgUrl": "/mnt/lg/cmn_data/channel_logo/major_logo_img/19586_202502221002_h356x200.png", "channelTypeId": 1,
                 "skipped": false, "Invisible": false, "favoriteIdxA": 250, "favoriteIdxB": 1, "favoriteIdxC": 250, "favoriteIdxD": 250},
                {"channelId": "1_32_2_2_0_0_0", "channelNumber": "2-2", "channelName": "BOUNCE", "imgUrl": "", "skipped": false, "Invisible": false},
                {"channelId": "1_32_9_1_0_0_0", "channelNumber": "9-1", "channelName": "Hidden", "skipped": true},
                {"channelId": "1_32_9_2_0_0_0", "channelNumber": "9-2", "channelName": "Unseen", "Invisible": true},
                {"channelId": "11_3_100_65535_0_0_0", "channelNumber": "100", "channelName": "", "imgUrl": "https://example.invalid/logo.png"}
            ]
        })
    }

    #[test]
    fn the_list_has_the_channels_lg_shows_in_the_order_it_gave() {
        let c = parse_channels(&list(), Source::Tuner).unwrap();
        assert_eq!(c.iter().map(|c| (c.number.as_str(), c.name.as_str())).collect::<Vec<_>>(), [("2-1", "WSB-HD"), ("2-2", "BOUNCE"), ("100", "100")], "skipped and invisible are out, a nameless one is its number");
        assert_eq!(c[0].id, "1_32_2_1_0_0_0");
        assert_eq!(c[0].logo.as_deref(), Some("/mnt/lg/cmn_data/channel_logo/major_logo_img/19586_202502221002_h356x200.png"));
        assert_eq!(c[1].logo, None, "an empty logo is none");
        assert_eq!(c[2].logo, None, "a web address is not a file of the set");
        assert_eq!((c[0].favourites, c[0].favourite(), c[1].favourite()), (0b10, true, false), "a place in group B; 250 is not a place");
    }

    #[test]
    fn channels_are_ordered_by_their_numbers_not_their_text() {
        let mk = |n: &str| Channel { id: n.into(), number: n.into(), name: n.into(), logo: None, source: Source::Tuner, mode: String::new(), favourites: 0 };
        let order: Vec<_> = merge(vec![mk("100"), mk("11-2"), mk("2-10"), mk("2-2"), mk("x"), mk("11-11")]).into_iter().map(|c| c.number).collect();
        assert_eq!(order, ["2-2", "2-10", "11-2", "11-11", "100", "x"]);
    }

    #[test]
    fn a_service_that_says_no_or_something_else_is_a_fault() {
        let no = json!({"returnValue": false, "errorCode": -106, "errorText": "No resource: "});
        assert_eq!(parse_channels(&no, Source::Tuner), Err(Fault::Refused { text: "No resource: ".into(), code: -106 }));
        assert_eq!(parse_programmes(&no, 0), Err(Fault::Refused { text: "No resource: ".into(), code: -106 }));
        assert_eq!(parse_channels(&json!({"returnValue": true}), Source::Tuner), Err(Fault::Shape("no channelList")));
        assert_eq!(parse_programmes(&json!({"returnValue": true}), 0), Err(Fault::Shape("no programList")));
        assert_eq!(parse_current(&no), None);
        assert_eq!(parse_last(&no), None);
    }

    /// Two programmes of a channel, an overlapping window's first and the next, as the set answered (EST on 2026-11-03: UTC minus five hours).
    fn events() -> Value {
        json!({
            "returnValue": true, "programListCount": 3, "dataSource": 0, "dataType": 0,
            "programList": [
                {"programId": "1_32_2_1_0_0_0_0003", "channelId": "1_32_2_1_0_0_0", "programName": "Jimmy Kimmel Live", "programMainTitle": "Jimmy Kimmel Live",
                 "description": "A late-night talk show.", "duration": 3720, "startTime": "2026,11,04,04,35,00", "endTime": "2026,11,04,05,37,00",
                 "localStartTime": "2026,11,03,23,35,00", "localEndTime": "2026,11,04,00,37,00", "isSubtitle": true, "isAD": false,
                 "rating": [{"dimension": [{"dim": 0, "dimRatingString": "TV TV-14"}, {"dim": 1, "dimRatingString": "D"}, {"dim": 2, "dimRatingString": "L"}]}]},
                {"programId": "1_32_2_1_0_0_0_0002", "channelId": "1_32_2_1_0_0_0", "programName": "WSB Tonight", "description": "News.", "duration": 2100,
                 "startTime": "2026,11,04,04,00,00", "endTime": "2026,11,04,04,35,00", "localStartTime": "2026,11,03,23,00,00", "localEndTime": "2026,11,03,23,35,00",
                 "rating": []},
                {"programId": "x", "channelId": "1_32_2_1_0_0_0", "programName": "No UTC", "localStartTime": "2026,11,04,00,37,00", "duration": 1800}
            ]
        })
    }

    #[test]
    fn programmes_are_instants_in_start_order_with_the_rating_and_flags_read() {
        let p = parse_programmes(&events(), -5 * 3600).unwrap();
        assert_eq!(p.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["WSB Tonight", "Jimmy Kimmel Live", "No UTC"]);
        let utc = |s: &str| parse_stamp(s, 0).unwrap();
        assert_eq!((p[0].start, p[0].end), (utc("2026,11,04,04,00,00"), utc("2026,11,04,04,35,00")));
        assert_eq!(p[0].minutes(), 35);
        assert_eq!(p[1].rating, "TV-14 DL");
        assert_eq!(p[0].rating, "");
        assert!(p[1].captions && !p[1].described && !p[0].captions);
        assert_eq!(p[1].text, "A late-night talk show.");
        assert_eq!(p[2].start, utc("2026,11,04,05,37,00"), "a programme with only its local time is read on the offset it is given");
        assert_eq!(p[2].end, p[2].start + 1_800_000, "and ends after its duration");
    }

    #[test]
    fn the_offset_is_read_from_the_pair_of_times_and_rounded_to_a_quarter_hour() {
        assert_eq!(offset_of(&events()), Some(-5 * 3600));
        let dst = json!({"programList": [{"startTime": "2026,07,04,03,00,00", "localStartTime": "2026,07,03,23,00,00"}]});
        assert_eq!(offset_of(&dst), Some(-4 * 3600));
        let odd = json!({"programList": [{"startTime": "2026,07,04,03,00,00", "localStartTime": "2026,07,04,08,30,40"}]});
        assert_eq!(offset_of(&odd), Some(5 * 3600 + 1800));
        assert_eq!(offset_of(&json!({"programList": [{"startTime": "2026,07,04,03,00,00"}]})), None);
    }

    #[test]
    fn a_stamp_goes_both_ways_on_any_clock() {
        let t = parse_stamp("2026,11,04,04,00,00", 0).unwrap();
        assert_eq!(stamp(t, -5 * 3600), "2026,11,03,23,00,00", "LG's local fields for the request");
        assert_eq!(parse_stamp("2026,11,03,23,00,00", -5 * 3600), Some(t));
        assert_eq!(stamp(t, 0), "2026,11,04,04,00,00");
        assert_eq!(stamp(t, 5 * 3600 + 1800), "2026,11,04,09,30,00");
        assert_eq!(parse_stamp("2026,13,04,04,00,00", 0), None);
        assert_eq!(parse_stamp("2026,11,04,25,00,00", 0), None);
        assert_eq!(parse_stamp("", 0), None);
        assert_eq!(parse_stamp("2026,11,04,04,00,00,1", 0), None);
    }

    #[test]
    fn the_requests_are_the_ones_lgs_own_clients_make() {
        let r = list_request(Source::Tuner);
        assert_eq!((r["dataType"].clone(), r["channelMode"].clone(), r["channelGroup"].clone()), (json!(0), json!(["Tuner"]), json!(["All"])));
        assert!(r["requestField"].as_array().unwrap().iter().any(|f| f == "imgUrl"));
        assert_eq!(list_request(Source::Internet)["channelMode"], json!(["IP"]));
        let from = parse_stamp("2026,11,04,04,00,00", 0).unwrap();
        let e = events_request(&["a", "b"], from, from + 6 * 3_600_000, -5 * 3600);
        assert_eq!(e, json!({"dataType": 0, "localStartTime": "2026,11,03,23,00,00", "localEndTime": "2026,11,04,05,00,00", "channelId": ["a", "b"]}));
        assert_eq!(tune_request("1_32_2_1_0_0_0"), json!({"id": "com.webos.app.livetv", "params": {"id": "channelChange", "channelId": "1_32_2_1_0_0_0", "sender": "livemenu", "type": "channels"}}));
        assert_eq!(last_request(false), json!({"sourceType": "INPUTSOURCE_RF", "subscribe": false}));
        assert_eq!(info_request("a"), json!({"dataType": 0, "channelId": "a"}));
        assert_eq!(now_next_request(&["a"]), json!({"dataType": 0, "channelId": ["a"]}));
    }

    #[test]
    fn now_and_next_come_from_the_sorted_list() {
        let p = parse_programmes(&events(), -5 * 3600).unwrap();
        let at = |s: &str| parse_stamp(s, 0).unwrap();
        let (now, next) = now_next(&p, at("2026,11,04,04,10,00"));
        assert_eq!((now.map(|p| p.title.as_str()), next.map(|p| p.title.as_str())), (Some("WSB Tonight"), Some("Jimmy Kimmel Live")));
        assert!((now.unwrap().progress(at("2026,11,04,04,10,00")) - 10.0 / 35.0).abs() < 1e-4);
        let (now, next) = now_next(&p, at("2026,11,04,03,00,00"));
        assert_eq!((now, next.map(|p| p.title.as_str())), (None, Some("WSB Tonight")), "before the first, nothing is on and the first is next");
        let (now, next) = now_next(&p, at("2026,11,04,05,50,00"));
        assert_eq!((now.map(|p| p.title.as_str()), next), (Some("No UTC"), None));
        assert_eq!(now_next(&p, at("2026,11,04,09,00,00")), (None, None));
        assert_eq!(now_next(&[], 5), (None, None));
    }

    #[test]
    fn the_last_and_the_current_channel_are_read_as_the_set_says_them() {
        assert_eq!(parse_last(&json!({"returnValue": true, "channelId": "1_32_2_1_0_0_0"})), Some("1_32_2_1_0_0_0".into()));
        assert_eq!(parse_last(&json!({"returnValue": true, "channelId": ""})), None);
        let c = parse_current(&json!({"returnValue": true, "channel": {"channelId": "1_32_2_1_0_0_0", "channelNumber": "2-1", "channelName": "WSB-HD", "channelTypeId": 1}}));
        assert_eq!(c, Some(Current { id: "1_32_2_1_0_0_0".into(), number: "2-1".into(), name: "WSB-HD".into() }));
        assert_eq!(parse_current(&json!({"returnValue": false, "errorCode": -106, "errorText": "No resource: "})), None);
        assert_eq!(current_request("", true), json!({"broadcastId": "", "subscribe": true}));
    }

    #[test]
    fn one_channels_info_is_the_channel_with_its_logo() {
        let info = json!({"returnValue": true, "channelList": [{"channelId": "1_32_2_1_0_0_0", "channelNumber": "2-1", "channelName": "WSB-HD", "channelMode": "Terrestrial",
            "imgUrl": "/mnt/lg/cmn_data/channel_logo/major_logo_img/19586_202502221002_h356x200.png", "skipped": false}]});
        let c = parse_channel_info(&info).unwrap();
        assert_eq!((c.number.as_str(), c.name.as_str(), c.source, c.mode.as_str()), ("2-1", "WSB-HD", Source::Tuner, "Terrestrial"));
        assert!(c.logo.is_some());
        let net = json!({"returnValue": true, "channelList": [{"channelId": "11_3_100_65535_0_0_0", "channelNumber": "100", "channelName": "LG Channels"}]});
        assert_eq!(parse_channel_info(&net).map(|c| c.source), Some(Source::Internet));
        assert_eq!(parse_channel_info(&json!({"returnValue": true, "channelList": []})), None);
    }
}
