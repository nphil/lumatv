//! News: RSS feeds fetched in order, parsed without an XML library, interleaved and de-duplicated.
//!
//! A port of `parseFeed`, `field`, `link`, `decode`, `imageOf` and `merge` in `NewsWidget.qml`. The QML regular
//! expressions become plain scans because the crate has no regex dependency; the places where the scans are stricter than
//! the QML (an element name must end at `>`, whitespace or `/`) are noted on the functions.

use std::collections::HashSet;
use std::time::Duration;

use fetch::{Fetch, FetchLimits};
use serde::{Deserialize, Serialize};

use crate::calendar;
use crate::{EXPIRE_MS, FeedError, REFRESH_MS, SCHEMA};

/// One feed: the tag and source label shown on its stories, and where it is fetched from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedSpec {
    /// Section label on the card, e.g. `WORLD`.
    pub tag: String,
    /// Publisher name, e.g. `BBC News`.
    pub src: String,
    /// The feed address (HTTPS).
    pub url: String,
}

/// Which feeds to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewsConfig {
    /// The primary feeds; their URLs are the cache key.
    pub feeds: Vec<FeedSpec>,
    /// Used only when the primary feeds give no story at all.
    pub alt: Vec<FeedSpec>,
    /// The most stories taken from one feed.
    pub per_feed: usize,
}

impl NewsConfig {
    /// The QML's defaults: BBC World and BBC U.S., NYT World and NYT U.S. as the fallback, 4 stories per feed.
    pub fn standard() -> NewsConfig {
        NewsConfig {
            feeds: default_feeds(),
            alt: default_alt_feeds(),
            per_feed: 4,
        }
    }
}

fn spec(tag: &str, src: &str, url: &str) -> FeedSpec {
    FeedSpec {
        tag: tag.to_owned(),
        src: src.to_owned(),
        url: url.to_owned(),
    }
}

/// The QML's primary feeds: BBC World, then BBC U.S. & Canada.
pub fn default_feeds() -> Vec<FeedSpec> {
    vec![
        spec("WORLD", "BBC News", "https://feeds.bbci.co.uk/news/world/rss.xml"),
        spec("U.S.", "BBC News", "https://feeds.bbci.co.uk/news/world/us_and_canada/rss.xml"),
    ]
}

/// The QML's fallback feeds: NYT World, then NYT U.S.
pub fn default_alt_feeds() -> Vec<FeedSpec> {
    vec![
        spec("WORLD", "The New York Times", "https://rss.nytimes.com/services/xml/rss/nyt/World.xml"),
        spec("U.S.", "The New York Times", "https://rss.nytimes.com/services/xml/rss/nyt/US.xml"),
    ]
}

/// One story as the card draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewsItem {
    pub tag: String,
    pub src: String,
    pub title: String,
    pub summary: String,
    pub link: String,
    /// Image URL exactly as the feed gives it (the card picks its own width), or empty.
    pub img: String,
    /// The feed's `pubDate` text.
    #[serde(rename = "pub")]
    pub pub_date: String,
    /// `pub_date` as epoch milliseconds; `null` when it cannot be read.
    #[serde(rename = "pubMs")]
    pub pub_ms: Option<u64>,
}

/// The stored and served news document: the on-disk `news.json`, which QML reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewsDoc {
    /// [`SCHEMA`].
    pub schema: u32,
    /// `now_ms` of the successful fetch.
    #[serde(rename = "fetchedAt")]
    pub fetched_at: u64,
    /// `fetched_at` + [`REFRESH_MS`].
    #[serde(rename = "staleAt")]
    pub stale_at: u64,
    /// `fetched_at` + [`EXPIRE_MS`].
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    /// URLs of `cfg.feeds` in order: the cache key. It is the configuration, not the source that answered, so it stays the
    /// same when the stories came from the fallback feeds.
    pub feeds: Vec<String>,
    pub items: Vec<NewsItem>,
}

/// Fetches the primary feeds one after the other and merges them; if that gives nothing, does the same with the fallback
/// feeds. A feed that fails or yields no story contributes an empty bucket. With no story at all the first transport error
/// is returned, or [`FeedError::Empty`] when every request succeeded.
pub fn refresh(fetch: &dyn Fetch, cfg: &NewsConfig, now_ms: u64) -> Result<NewsDoc, FeedError> {
    let mut first_error = None;
    let mut items = load(fetch, &cfg.feeds, cfg.per_feed, &mut first_error);
    if items.is_empty() {
        items = load(fetch, &cfg.alt, cfg.per_feed, &mut first_error);
    }
    if items.is_empty() {
        return Err(first_error.unwrap_or(FeedError::Empty));
    }
    Ok(NewsDoc {
        schema: SCHEMA,
        fetched_at: now_ms,
        stale_at: now_ms.saturating_add(REFRESH_MS),
        expires_at: now_ms.saturating_add(EXPIRE_MS),
        feeds: cfg.feeds.iter().map(|f| f.url.clone()).collect(),
        items,
    })
}

/// Fetches, parses and merges one list of feeds, remembering the first transport error.
fn load(
    fetch: &dyn Fetch,
    feeds: &[FeedSpec],
    per_feed: usize,
    first_error: &mut Option<FeedError>,
) -> Vec<NewsItem> {
    // Feeds are well under 512 KiB; the cap bounds a hostile or broken answer.
    let limits = FetchLimits {
        total: Duration::from_secs(10),
        max_bytes: 512 * 1024,
    };
    let buckets = feeds
        .iter()
        .map(|feed| match fetch.get(&feed.url, limits) {
            Ok(body) => parse_feed(&String::from_utf8_lossy(&body), feed, per_feed),
            Err(e) => {
                first_error.get_or_insert(FeedError::Fetch(e));
                Vec::new()
            }
        })
        .collect();
    merge(buckets)
}

/// The first `per_feed` stories of an RSS document, in document order. Every `<item>`..`</item>` (case-sensitive) is read; an
/// item whose decoded title is empty is skipped and does not count toward the limit.
pub fn parse_feed(xml: &str, feed: &FeedSpec, per_feed: usize) -> Vec<NewsItem> {
    let mut out = Vec::new();
    let mut rest = xml;
    while out.len() < per_feed {
        let Some(start) = rest.find("<item>") else { break };
        let after = &rest[start + "<item>".len()..];
        let Some(end) = after.find("</item>") else { break };
        let block = &after[..end];
        rest = &after[end + "</item>".len()..];
        let title = field(block, "title");
        if title.is_empty() {
            continue;
        }
        let pub_date = field(block, "pubDate");
        out.push(NewsItem {
            tag: feed.tag.clone(),
            src: feed.src.clone(),
            title,
            summary: field(block, "description"),
            link: link(block),
            img: image_of(block),
            pub_ms: parse_rfc2822_ms(&pub_date),
            pub_date,
        });
    }
    out
}

/// Interleaves the buckets by index (bucket 0 story 0, bucket 1 story 0, bucket 0 story 1, ...) and drops a story whose
/// lower-cased title was already taken.
pub fn merge(buckets: Vec<Vec<NewsItem>>) -> Vec<NewsItem> {
    let mut iters: Vec<_> = buckets.into_iter().map(Vec::into_iter).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    loop {
        let mut more = false;
        for it in &mut iters {
            if let Some(item) = it.next() {
                more = true;
                if seen.insert(item.title.to_lowercase()) {
                    out.push(item);
                }
            }
        }
        if !more {
            return out;
        }
    }
}

/// Turns feed markup into plain text, in the QML's order: CDATA markers removed, tags stripped, the named entities
/// `&lt; &gt; &quot; &#39; &#039; &apos; &nbsp;` replaced, then every other numeric entity (`&#8217;`, `&#x2019;`; the QML
/// leaves these, this is an intended superset; an invalid code point stays as written), `&amp;` last so `&amp;lt;` becomes
/// `&lt;` and not `<`, and finally whitespace runs collapsed to one space and the ends trimmed.
pub fn decode_text(s: &str) -> String {
    let s = s.replace("<![CDATA[", "").replace("]]>", "");
    let s = strip_tags(&s)
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    let s = numeric_entities(&s).replace("&amp;", "&");
    collapse_whitespace(&s)
}

/// Removes every `<...>` that has at least one character between the brackets (the QML's `<[^>]+>`).
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt + 1..];
        match after.find('>') {
            Some(gt) if gt > 0 => {
                out.push_str(&rest[..lt]);
                rest = &after[gt + 1..];
            }
            _ => {
                // A lone `<` (no `>` after it, or `<>`) is text.
                out.push_str(&rest[..=lt]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replaces `&#NNN;` and `&#xHH;` with the character; anything that is not a valid scalar value is left as written.
fn numeric_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("&#") {
        out.push_str(&rest[..at]);
        let body = &rest[at + 2..];
        let decoded = body
            .find(';')
            .and_then(|semi| entity_char(&body[..semi]).map(|ch| (ch, semi)));
        match decoded {
            Some((ch, semi)) => {
                out.push(ch);
                rest = &body[semi + 1..];
            }
            None => {
                out.push_str("&#");
                rest = body;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character of a numeric entity's digits (`8217` or `x2019`); `None` for anything but plain digits, for zero and for
/// code points that are not scalar values.
fn entity_char(digits: &str) -> Option<char> {
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) if (1..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            u32::from_str_radix(hex, 16).ok()?
        }
        None if (1..=7).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()) => {
            digits.parse().ok()?
        }
        _ => return None,
    };
    if code == 0 { None } else { char::from_u32(code) }
}

/// One space per run of whitespace (JavaScript's `\s`, which also counts U+FEFF), no leading or trailing space.
fn collapse_whitespace(s: &str) -> String {
    let is_ws = |c: char| c.is_whitespace() || c == '\u{feff}';
    s.split(is_ws).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The decoded text of the first `<tag ...>..</tag>` of `block`, or empty. The element name must be followed by `>`,
/// whitespace or `/`, so `<title` never matches `<titles>` (the QML pattern would).
fn field(block: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut from = 0;
    while let Some(rel) = block[from..].find(&open) {
        let name_end = from + rel + open.len();
        from = name_end;
        match block[name_end..].chars().next() {
            Some(c) if c == '>' || c == '/' || c.is_whitespace() => {}
            _ => continue,
        }
        let Some(gt) = block[name_end..].find('>') else { break };
        let inner = &block[name_end + gt + 1..];
        return inner.find(&close).map(|e| decode_text(&inner[..e])).unwrap_or_default();
    }
    String::new()
}

/// The decoded text of a bare `<link>`..`</link>` (no attributes), so Atom's `<atom:link href=...>` never matches.
fn link(block: &str) -> String {
    let Some(start) = block.find("<link>") else { return String::new() };
    let inner = &block[start + "<link>".len()..];
    inner.find("</link>").map(|e| decode_text(&inner[..e])).unwrap_or_default()
}

/// The widest `<media:content>` / `<media:thumbnail>` image that is at most 1200 wide (a later tag replaces the best only
/// with a strictly larger width; a tag without `width` counts as 0), else the narrowest wider one, else empty. The URL is
/// left as the feed gives it except for `&amp;`, which feeds use to escape `&`; the card rewrites the width itself.
fn image_of(block: &str) -> String {
    let mut best: Option<(u64, &str)> = None;
    let mut over: Option<(u64, &str)> = None;
    let mut rest = block;
    while let Some(at) = rest.find("<media:") {
        let tail = &rest[at + "<media:".len()..];
        let skip = if tail.starts_with("content") {
            "content".len()
        } else if tail.starts_with("thumbnail") {
            "thumbnail".len()
        } else {
            rest = tail;
            continue;
        };
        let tail = &tail[skip..];
        let Some(end) = tail.find('>') else { break };
        let attrs = &tail[..end];
        rest = &tail[end..];
        let Some(url) = attribute(attrs, "url").filter(|u| !u.is_empty()) else { continue };
        let width = attribute(attrs, "width").map_or(0, number_or_zero);
        if width <= 1200 {
            if best.is_none_or(|(w, _)| width > w) {
                best = Some((width, url));
            }
        } else if over.is_none_or(|(w, _)| width < w) {
            over = Some((width, url));
        }
    }
    best.or(over).map(|(_, u)| u.replace("&amp;", "&")).unwrap_or_default()
}

/// A run of digits as a number (saturating); anything else is 0, as for an absent `width`.
fn number_or_zero(s: &str) -> u64 {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    s.parse().unwrap_or(u64::MAX)
}

/// The value of `name="..."` or `name='...'` in the attribute text of a start tag; `name` must start the text or follow
/// whitespace.
fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(rel) = attrs[from..].find(name) {
        let at = from + rel;
        from = at + name.len();
        if attrs[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let Some(value) = attrs[from..].strip_prefix('=') else { continue };
        let Some(quote) = value.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let value = &value[1..];
        if let Some(end) = value.find(quote) {
            return Some(&value[..end]);
        }
    }
    None
}

/// An RFC 2822 date (`Sun, 25 Oct 2026 00:10:07 +0000`) as Unix epoch milliseconds.
///
/// Accepted: an optional `Www, `, `D Mon YYYY HH:MM[:SS]` with a four-digit year, then `GMT`, `UT`, `UTC`, `Z`, `+hhmm` /
/// `-hhmm` or one of EST/EDT/CST/CDT/MST/MDT/PST/PDT. Month and zone names are case-insensitive. Anything else, and any
/// instant before 1970, is `None`.
pub fn parse_rfc2822_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = match s.split_once(',') {
        Some((weekday, rest)) => {
            let weekday = weekday.trim();
            if weekday.len() != 3 || !weekday.bytes().all(|b| b.is_ascii_alphabetic()) {
                return None;
            }
            rest
        }
        None => s,
    };
    let mut parts = s.split_whitespace();
    let day = parts.next()?;
    let month = parts.next()?;
    let year = parts.next()?;
    let time = parts.next()?;
    let zone = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let day = small_number(day, 1..=2)?;
    let month = month_number(month)?;
    if year.len() != 4 {
        return None;
    }
    let year = i64::from(small_number(year, 4..=4)?);
    if year < 1970 || day == 0 || day > calendar::days_in_month(year, month) {
        return None;
    }

    let mut clock = time.split(':');
    let hour = small_number(clock.next()?, 2..=2)?;
    let minute = small_number(clock.next()?, 2..=2)?;
    let second = match clock.next() {
        Some(sec) => small_number(sec, 2..=2)?,
        None => 0,
    };
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let zone_seconds = zone_offset_seconds(zone)?;
    let days = calendar::days_from_civil(year, month, day);
    let seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second) - zone_seconds;
    u64::try_from(seconds).ok()?.checked_mul(1000)
}

/// A string of ASCII digits whose length is in `len`, as a number.
fn small_number(s: &str, len: std::ops::RangeInclusive<usize>) -> Option<u32> {
    if !len.contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    MONTHS
        .iter()
        .position(|m| name.eq_ignore_ascii_case(m))
        .map(|i| i as u32 + 1)
}

/// Seconds east of UTC for an RFC 2822 zone.
fn zone_offset_seconds(zone: &str) -> Option<i64> {
    const NAMED: [(&str, i64); 12] = [
        ("GMT", 0),
        ("UT", 0),
        ("UTC", 0),
        ("Z", 0),
        ("EST", -5),
        ("EDT", -4),
        ("CST", -6),
        ("CDT", -5),
        ("MST", -7),
        ("MDT", -6),
        ("PST", -8),
        ("PDT", -7),
    ];
    if let Some((_, hours)) = NAMED.iter().find(|(n, _)| zone.eq_ignore_ascii_case(n)) {
        return Some(hours * 3_600);
    }
    let sign = match *zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = &zone[1..];
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = digits[2..].parse().ok()?;
    if minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3_600 + minutes * 60))
}
