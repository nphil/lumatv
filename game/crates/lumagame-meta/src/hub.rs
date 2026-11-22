//! What a game's page shows beyond its artwork, from public sources that need no key: the Steam store (description, scores,
//! screenshots, trailers, achievements), Steam news, Steam Deck compatibility, player reviews and PCGamingWiki's facts (HDR,
//! ultrawide, controllers). Each source stands alone: one that fails or has nothing leaves its part of the [`Hub`] empty, and the page
//! hides an empty part. Everything is fetched on a worker, paced per host, and kept in the bounded [`Cache`] so a page works offline.

use crate::{Cache, providers::{Client, Transport}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex, time::{Duration, Instant}};

const BODY_LIMIT: usize = 2 << 20;
/// A cached page older than this is fetched again; offline it is still served.
pub const FRESH_SECONDS: u64 = 7 * 24 * 3600;
pub const STEAM_CREDIT: &str = "Store data, news and reviews from Steam (store.steampowered.com). Steam is a trademark of Valve Corporation.";
pub const PCGW_CREDIT: &str = "Game facts from PCGamingWiki (pcgamingwiki.com), CC BY-NC-SA 3.0.";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hub {
    /// When it was fetched, in Unix seconds.
    pub fetched_at: u64,
    pub description: String,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub release: Option<String>,
    pub genres: Vec<String>,
    pub categories: Vec<String>,
    pub metacritic: Option<u32>,
    pub review: Option<Review>,
    pub controller: Option<String>,
    pub dlc: u32,
    pub deck: Option<Deck>,
    pub achievements: Achievements,
    pub news: Vec<News>,
    pub screenshots: Vec<String>,
    pub trailers: Vec<Trailer>,
    pub facts: Vec<Fact>,
    /// Who the data is from, for the details section; a source that gave nothing is not named.
    pub credits: Vec<String>,
}
impl Hub {
    pub fn is_empty(&self) -> bool {
        self.credits.is_empty()
    }
    pub fn fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) < FRESH_SECONDS
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// Steam's own words: "Very Positive".
    pub label: String,
    pub percent: Option<u32>,
    pub count: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Deck {
    Verified,
    Playable,
    Unsupported,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Achievements {
    pub total: u32,
    /// The ones the store page names, with how many players have each where the percentage list pairs up with them.
    pub shown: Vec<Achievement>,
    /// The smallest share of players that has any one of them, in tenths of a percent.
    pub rarest: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Achievement {
    /// The name the owner's own unlocked list uses, when the percentages named it.
    pub api: Option<String>,
    pub name: String,
    pub icon: String,
    /// The share of players that has it, in tenths of a percent.
    pub permille: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct News {
    pub title: String,
    pub author: String,
    pub date: u64,
    pub url: String,
    pub summary: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trailer {
    pub title: String,
    pub thumb: String,
    /// An adaptive stream (HLS), opened outside the app: nothing here plays by itself.
    pub url: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

/// A least gap between two requests to one host, so a page opening never hammers a public service.
pub struct Pacing {
    gap: Duration,
    next: Mutex<BTreeMap<String, Instant>>,
}
impl Pacing {
    pub fn new(gap: Duration) -> Self {
        Self { gap, next: Mutex::new(BTreeMap::new()) }
    }
    fn wait(&self, host: &str) {
        let slot = {
            let Ok(mut next) = self.next.lock() else { return };
            let now = Instant::now();
            let slot = next.get(host).map_or(now, |t| (*t).max(now));
            next.insert(host.to_owned(), slot + self.gap);
            slot
        };
        std::thread::sleep(slot.saturating_duration_since(Instant::now()));
    }
}

/// What identifies the game. A Steam app id is an identity; a title only names a PCGamingWiki page, and the page must carry exactly
/// that title.
pub struct Lookup<'a> {
    pub steam: Option<u64>,
    pub title: &'a str,
}

/// The page for a game: the cache when it is fresh, else the sources, else the cache however old, else nothing.
pub fn load<T: Transport>(client: &Client<T>, pace: &Pacing, cache: &Mutex<Option<Cache>>, lookup: &Lookup<'_>, now: u64) -> Option<Hub> {
    let key = match lookup.steam {
        Some(id) => format!("hub:steam:{id}"),
        None => format!("hub:title:{}", normal(lookup.title)),
    };
    let saved = cache.lock().ok().and_then(|mut c| c.as_mut().and_then(|c| c.get(&key).ok().flatten())).and_then(|(_, bytes)| serde_json::from_slice::<Hub>(&bytes).ok());
    if let Some(hub) = saved.as_ref().filter(|h| h.fresh(now)) {
        return Some(hub.clone());
    }
    let hub = fetch(client, pace, lookup, now);
    if hub.is_empty() {
        return saved;
    }
    if let (Ok(mut cache), Ok(bytes)) = (cache.lock(), serde_json::to_vec(&hub)) {
        if let Some(cache) = cache.as_mut() {
            let _ = cache.put(&key, &bytes);
        }
    }
    Some(hub)
}

pub fn fetch<T: Transport>(client: &Client<T>, pace: &Pacing, lookup: &Lookup<'_>, now: u64) -> Hub {
    let mut hub = Hub { fetched_at: now, ..Hub::default() };
    let mut name = lookup.title.to_owned();
    let mut steam = false;
    if let Some(id) = lookup.steam {
        if let Some(store) = get(client, pace, &format!("https://store.steampowered.com/api/appdetails?appids={id}&l=english"), &[]).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            if let Some(found) = store_details(&mut hub, &store, id) {
                name = found;
                steam = true;
            }
        }
        if let Some(deck) = get(client, pace, &format!("https://store.steampowered.com/saleaction/ajaxgetdeckappcompatibilityreport?nAppID={id}"), &[]).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            hub.deck = deck_category(&deck);
        }
        hub.review = reviews(client, pace, id);
        if let Some(news) = get(client, pace, &format!("https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid={id}&count=8&maxlength=600&format=json"), &[]).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            hub.news = news_items(&news);
        }
        if let Some(shares) = get(client, pace, &format!("https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?gameid={id}&format=json"), &[]).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            rarity(&mut hub.achievements, &shares);
        }
        if steam || hub.deck.is_some() || hub.review.is_some() || !hub.news.is_empty() {
            push_credit(&mut hub, STEAM_CREDIT);
        }
    }
    if !name.is_empty() {
        let url = format!("https://www.pcgamingwiki.com/w/api.php?action=parse&page={}&prop=wikitext&format=json&redirects=1", encode(&name));
        if let Some(page) = get(client, pace, &url, &[]).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            let text = page.pointer("/parse/wikitext/*").and_then(Value::as_str).unwrap_or_default();
            let title = page.pointer("/parse/title").and_then(Value::as_str).unwrap_or_default();
            if page_is(text, title, lookup) {
                hub.facts = facts(text);
                if !hub.facts.is_empty() {
                    push_credit(&mut hub, PCGW_CREDIT);
                }
            }
        }
    }
    hub
}

fn push_credit(hub: &mut Hub, credit: &str) {
    if !hub.credits.iter().any(|c| c == credit) {
        hub.credits.push(credit.to_owned());
    }
}

fn get<T: Transport>(client: &Client<T>, pace: &Pacing, url: &str, headers: &[(&str, &str)]) -> Option<Vec<u8>> {
    pace.wait(url.split('/').nth(2).unwrap_or_default());
    let response = client.get(url, headers, BODY_LIMIT).ok()?;
    (200..300).contains(&response.status).then_some(response.body)
}

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or_default()
}
fn names(value: &Value, pointer: &str, key: &str, max: usize) -> Vec<String> {
    value.pointer(pointer).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.get(key).and_then(Value::as_str)).map(plain).filter(|s| !s.is_empty()).take(max).collect()).unwrap_or_default()
}
fn strings(value: &Value, pointer: &str) -> Vec<String> {
    value.pointer(pointer).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(plain).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}
fn https(url: &str) -> bool {
    url.starts_with("https://") && !url.chars().any(char::is_control)
}

/// The store's own record of the game. Returns the game's name.
fn store_details(hub: &mut Hub, store: &Value, id: u64) -> Option<String> {
    let data = store.get(id.to_string())?.get("data").filter(|_| store.pointer(&format!("/{id}/success")).and_then(Value::as_bool) == Some(true))?;
    hub.description = plain(text(data, "/short_description"));
    hub.developers = strings(data, "/developers");
    hub.publishers = strings(data, "/publishers");
    hub.release = Some(text(data, "/release_date/date")).filter(|d| !d.is_empty() && data.pointer("/release_date/coming_soon").and_then(Value::as_bool) != Some(true)).map(str::to_owned);
    hub.genres = names(data, "/genres", "description", 6);
    hub.categories = names(data, "/categories", "description", 12);
    hub.metacritic = data.pointer("/metacritic/score").and_then(Value::as_u64).filter(|s| (1..=100).contains(s)).map(|s| s as u32);
    hub.controller = match text(data, "/controller_support") {
        "full" => Some("Full controller support".to_owned()),
        "partial" => Some("Partial controller support".to_owned()),
        _ => None,
    };
    hub.dlc = data.pointer("/dlc").and_then(Value::as_array).map_or(0, |d| d.len() as u32);
    hub.screenshots = data.pointer("/screenshots").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.get("path_full").and_then(Value::as_str)).filter(|u| https(u)).take(12).map(str::to_owned).collect()).unwrap_or_default();
    hub.trailers = data.pointer("/movies").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(|m| {
            // An adaptive stream: the TV's own player takes either; the HLS one is the one every webOS build opens.
            let url = ["/hls_h264", "/dash_h264", "/dash_av1"].iter().map(|p| text(m, p)).find(|u| https(u))?;
            Some(Trailer { title: plain(text(m, "/name")), thumb: Some(text(m, "/thumbnail")).filter(|t| https(t)).unwrap_or_default().to_owned(), url: url.to_owned() })
        }).take(6).collect()
    }).unwrap_or_default();
    hub.achievements.total = data.pointer("/achievements/total").and_then(Value::as_u64).unwrap_or(0) as u32;
    hub.achievements.shown = data.pointer("/achievements/highlighted").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(|h| {
            let (name, icon) = (plain(text(h, "/name")), text(h, "/path"));
            (!name.is_empty() && https(icon)).then(|| Achievement { api: None, name, icon: icon.to_owned(), permille: None })
        }).collect()
    }).unwrap_or_default();
    Some(text(data, "/name").to_owned())
}

fn deck_category(report: &Value) -> Option<Deck> {
    match report.pointer("/results/resolved_category").and_then(Value::as_u64)? {
        3 => Some(Deck::Verified),
        2 => Some(Deck::Playable),
        1 => Some(Deck::Unsupported),
        _ => None,
    }
}

/// Steam's review summary. The JSON endpoint answers some networks with a redirect to the store's front page; [`Client::get`]
/// follows it, and a reply that is not the summary falls back to the words and share the store page itself prints.
fn reviews<T: Transport>(client: &Client<T>, pace: &Pacing, id: u64) -> Option<Review> {
    let json = get(client, pace, &format!("https://store.steampowered.com/appreviews/{id}?json=1&num_per_page=0&language=all&purchase_type=all"), &[])
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| {
            let s = v.get("query_summary")?;
            let (positive, total) = (s.get("total_positive")?.as_u64()?, s.get("total_reviews")?.as_u64()?);
            let label = s.get("review_score_desc")?.as_str()?.to_owned();
            (total > 0 && !label.is_empty() && !label.starts_with("No ")).then(|| Review { label, percent: Some((positive * 100 / total) as u32), count: total })
        });
    json.or_else(|| {
        let page = get(client, pace, &format!("https://store.steampowered.com/app/{id}/?l=english"), &[("Cookie", "birthtime=631152001; lastagecheckage=1-January-1990; wants_mature_content=1")])?;
        review_from_page(&String::from_utf8_lossy(&page))
    })
}
fn review_from_page(html: &str) -> Option<Review> {
    let at = html.find("game_review_summary")?;
    let label = html[at..].split_once("\">")?.1.split('<').next()?.trim().to_owned();
    if label.is_empty() || label.starts_with("No ") || label.contains(' ') && !label.chars().next()?.is_uppercase() {
        return None;
    }
    let tip = html.split("data-tooltip-html=\"").skip(1).map(|t| t.split('"').next().unwrap_or_default()).find(|t| t.contains("user reviews in your language"))?;
    let (percent, rest) = tip.split_once("% of the ")?;
    let count = rest.split(' ').next()?.replace(',', "").parse().ok()?;
    Some(Review { label, percent: percent.trim().parse().ok(), count })
}

fn news_items(news: &Value) -> Vec<News> {
    news.pointer("/appnews/newsitems").and_then(Value::as_array).map(|items| {
        items.iter().filter_map(|n| {
            let (title, url) = (plain(text(n, "/title")), text(n, "/url"));
            (!title.is_empty() && https(url)).then(|| News {
                title,
                author: plain(text(n, "/author")),
                date: n.get("date").and_then(Value::as_u64).unwrap_or(0),
                url: url.to_owned(),
                summary: clip(&plain(text(n, "/contents")), 360),
            })
        }).take(6).collect()
    }).unwrap_or_default()
}

/// Pairs the store's named achievements with the share of players that has each. The two lists use different names (a display name
/// against an internal one), so a pair is made only where the icon file's name or the display name equals the internal name once
/// its `ACH.` or `ACH_` prefix is dropped.
fn rarity(achievements: &mut Achievements, shares: &Value) {
    let list: Vec<(String, u32)> = shares.pointer("/achievementpercentages/achievements").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_owned();
            let percent = match s.get("percent")? {
                Value::String(p) => p.parse::<f64>().ok()?,
                other => other.as_f64()?,
            };
            Some((name, (percent * 10.0).round() as u32))
        }).collect()
    }).unwrap_or_default();
    achievements.rarest = list.iter().map(|(_, p)| *p).min();
    let squash = |s: &str| s.trim_start_matches("ACH.").trim_start_matches("ACH_").chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    for shown in &mut achievements.shown {
        let stem = shown.icon.rsplit('/').next().unwrap_or_default().rsplit_once('.').map_or("", |(stem, _)| stem);
        let (stem, display) = (squash(stem), squash(&shown.name));
        if let Some((api, permille)) = list.iter().find(|(api, _)| { let api = squash(api); api == stem || api == display }) {
            shown.api = Some(api.clone());
            shown.permille = Some(*permille);
        }
    }
}

/// Whether the wiki page is this game: its infobox lists the Steam app id, or (for a game without one) its title is the game's.
fn page_is(wikitext: &str, title: &str, lookup: &Lookup<'_>) -> bool {
    match lookup.steam {
        Some(id) => wikitext.lines().find_map(|l| l.trim().strip_prefix("|steam appid")).is_some_and(|rest| {
            rest.trim_start_matches([' ', '=']).split(',').any(|v| v.trim() == id.to_string())
        }),
        None => !title.is_empty() && normal(title) == normal(lookup.title),
    }
}

/// The wiki's yes/no/limited answers about the picture and the controls. Only an answer is a fact; an unknown is left out.
fn facts(wikitext: &str) -> Vec<Fact> {
    const VIDEO: [(&str, &str); 6] =
        [("ultrawidescreen", "Ultrawide"), ("4k ultra hd", "4K"), ("hdr", "HDR"), ("60 fps", "60 FPS"), ("120 fps", "120 FPS and above"), ("fov", "Field of view setting")];
    const INPUT: [(&str, &str); 5] = [
        ("full controller", "Full controller support"),
        ("controller remap", "Controller remapping"),
        ("playstation controllers", "PlayStation controllers"),
        ("dualsense adaptive trigger support", "Adaptive triggers"),
        ("haptic feedback", "Haptic feedback"),
    ];
    let mut out = Vec::new();
    for (template, keys) in [("Video", &VIDEO[..]), ("Input", &INPUT[..])] {
        let params = template_params(wikitext, template);
        for (key, label) in keys {
            let value = match params.get(*key).map(String::as_str) {
                Some("true") => "Yes",
                Some("false") => "No",
                Some("limited") => "Limited",
                Some("hackable") => "With a fix or mod",
                Some("always on") => "Always on",
                _ => continue,
            };
            out.push(Fact { label: (*label).to_owned(), value: value.to_owned() });
        }
    }
    out
}
/// The `|key = value` lines of one infobox-style template, until its closing `}}` line.
fn template_params(wikitext: &str, name: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    let mut inside = false;
    for line in wikitext.lines() {
        let line = line.trim();
        if !inside {
            inside = line.strip_prefix("{{").is_some_and(|rest| rest.trim() == name);
        } else if line == "}}" {
            break;
        } else if let Some((key, value)) = line.strip_prefix('|').and_then(|l| l.split_once('=')) {
            params.insert(key.trim().to_ascii_lowercase(), value.trim().to_ascii_lowercase());
        }
    }
    params
}

/// Text from the store's markup: tags and bracket codes dropped, entities read, white space collapsed.
pub fn plain(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(c) = rest.chars().next() {
        let skip = match c {
            '<' => rest.find('>').map(|i| (i + 1, separator(&rest[1..i]))),
            '[' => rest.find(']').filter(|i| *i <= 200 && is_code(&rest[1..*i])).map(|i| (i + 1, separator(&rest[1..i]))),
            '{' if rest.starts_with("{STEAM") => rest.find(|c: char| c.is_whitespace()).or(Some(rest.len())).map(|i| (i, "")),
            '&' => rest.find(';').filter(|i| *i <= 8).and_then(|i| {
                let entity = match &rest[1..i] {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" | "#39" | "#x27" => "'",
                    "nbsp" => " ",
                    "trade" => "™",
                    "reg" => "®",
                    _ => return None,
                };
                Some((i + 1, entity))
            }),
            _ => None,
        };
        match skip {
            Some((len, with)) => {
                out.push_str(with);
                rest = &rest[len..];
            }
            None => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end_matches(['.', ',', ';', ':']))
}
fn normal(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}
fn encode(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// "12 Mar 2025", for a Unix time in seconds. The calendar is the civil one in UTC.
pub fn date(unix: u64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}
/// What a tag or bracket code leaves between the words around it: a break for block-level ones, nothing for inline ones.
fn separator(code: &str) -> &'static str {
    let name = code.trim_matches('/').split([' ', '=']).next().unwrap_or_default().to_ascii_lowercase();
    if matches!(name.as_str(), "br" | "p" | "li" | "div" | "ul" | "ol" | "list" | "*" | "h1" | "h2" | "h3" | "h4" | "quote" | "img") { " " } else { "" }
}
/// A bracket code of Steam's news markup: `[b]`, `[/url]`, `[img=...]`. Square brackets in ordinary words are not one.
fn is_code(code: &str) -> bool {
    let name = code.trim_start_matches('/').split([' ', '=']).next().unwrap_or_default().to_ascii_lowercase();
    matches!(
        name.as_str(),
        "b" | "i" | "u" | "s" | "url" | "img" | "list" | "olist" | "*" | "h1" | "h2" | "h3" | "quote" | "spoiler" | "strike" | "code" | "p" | "hr" | "table" | "tr" | "td" | "th" | "noparse" | "previewyoutube" | "carousel" | "expand" | "dynamiclink"
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Result, providers::{Request, Response}, secrets::Secrets};
    use std::sync::Arc;

    /// Replies by URL substring, in any order; a URL nobody answers is a dead network.
    struct Fixed(Vec<(&'static str, Response)>, Arc<Mutex<Vec<String>>>);
    impl Transport for Fixed {
        fn execute(&self, request: Request) -> Result<Response> {
            self.1.lock().unwrap().push(request.url.clone());
            match self.0.iter().find(|(part, _)| request.url.contains(part)) {
                Some((_, r)) => Ok(Response { status: r.status, body: r.body.clone(), location: r.location.clone() }),
                None => Err(Error::Invalid("offline")),
            }
        }
    }
    fn ok(body: &str) -> Response {
        Response::new(200, body.as_bytes().to_vec())
    }
    fn served(replies: Vec<(&'static str, Response)>) -> (Client<Fixed>, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (Client::with_transport(Fixed(replies, seen.clone()), Secrets::default()), seen)
    }
    fn pace() -> Pacing {
        Pacing::new(Duration::ZERO)
    }
    const STORE: &str = r#"{"620":{"success":true,"data":{"name":"Portal 2","short_description":"The &quot;Perpetual Testing Initiative&quot; has been <b>expanded</b>.",
        "developers":["Valve"],"publishers":["Valve"],"controller_support":"full","dlc":[323180],"release_date":{"coming_soon":false,"date":"18 Apr, 2011"},
        "metacritic":{"score":95},"genres":[{"description":"Action"}],"categories":[{"description":"Single-player"},{"description":"Co-op"}],
        "screenshots":[{"path_full":"https://shared.akamai.steamstatic.com/a.1920x1080.jpg"},{"path_full":"http://insecure/b.jpg"}],
        "movies":[{"name":"E3 Demo","thumbnail":"https://cdn/t.jpg","hls_h264":"https://video/hls.m3u8","dash_h264":"https://video/d.mpd"}],
        "achievements":{"total":51,"highlighted":[{"name":"Wake Up Call","path":"https://cdn/apps/620/SURVIVE_CONTAINER_RIDE.jpg"},{"name":"Nameless","path":"https://cdn/apps/620/OTHER.jpg"}]}}}}"#;
    const SHARES: &str = r#"{"achievementpercentages":{"achievements":[{"name":"ACH.SURVIVE_CONTAINER_RIDE","percent":"74.0"},{"name":"ACH.RARE","percent":0.4}]}}"#;
    const NEWS: &str = r#"{"appnews":{"newsitems":[{"title":"Update 1.2","url":"https://store/news/1","author":"Valve","date":1700000000,"contents":"[b]Fixes[/b] for <i>co-op</i> &amp; menus. {STEAM_CLAN_IMAGE}/x.png"},{"title":"No link","url":"javascript:1","contents":""}]}}"#;
    const DECK: &str = r#"{"success":1,"results":{"resolved_category":3}}"#;
    const WIKI: &str = r#"{"parse":{"title":"Portal 2","wikitext":{"*":"{{Infobox game\n|steam appid  = 620\n}}\n{{Video\n|ultrawidescreen = true\n|hdr = unknown\n|60 fps = true\n|fov = hackable\n}}\n{{Input\n|full controller = true\n|controller remap = false\n}}"}}}"#;
    const REVIEWS: &str = r#"{"success":1,"query_summary":{"review_score_desc":"Overwhelmingly Positive","total_positive":970,"total_negative":30,"total_reviews":1000}}"#;
    fn all() -> Vec<(&'static str, Response)> {
        vec![("appdetails", ok(STORE)), ("ajaxgetdeck", ok(DECK)), ("GetNewsForApp", ok(NEWS)), ("GetGlobalAchievement", ok(SHARES)), ("appreviews", ok(REVIEWS)), ("pcgamingwiki", ok(WIKI))]
    }

    #[test]
    fn a_steam_game_gets_every_part_from_its_keyless_sources() {
        let (client, _) = served(all());
        let hub = fetch(&client, &pace(), &Lookup { steam: Some(620), title: "Portal 2" }, 1000);
        assert_eq!(hub.description, "The \"Perpetual Testing Initiative\" has been expanded.");
        assert_eq!((hub.developers.as_slice(), hub.release.as_deref(), hub.metacritic, hub.dlc), (&["Valve".to_owned()][..], Some("18 Apr, 2011"), Some(95), 1));
        assert_eq!(hub.controller.as_deref(), Some("Full controller support"));
        assert_eq!(hub.screenshots, ["https://shared.akamai.steamstatic.com/a.1920x1080.jpg"], "only https images");
        assert_eq!(hub.trailers, [Trailer { title: "E3 Demo".into(), thumb: "https://cdn/t.jpg".into(), url: "https://video/hls.m3u8".into() }]);
        assert_eq!(hub.deck, Some(Deck::Verified));
        assert_eq!(hub.review, Some(Review { label: "Overwhelmingly Positive".into(), percent: Some(97), count: 1000 }));
        assert_eq!(hub.news.len(), 1, "a news item without a web link is dropped");
        assert_eq!(hub.news[0].summary, "Fixes for co-op & menus.");
        assert_eq!((hub.achievements.total, hub.achievements.rarest), (51, Some(4)));
        assert_eq!(hub.achievements.shown[0].permille, Some(740));
        assert_eq!(hub.achievements.shown[0].api.as_deref(), Some("ACH.SURVIVE_CONTAINER_RIDE"));
        assert_eq!(hub.achievements.shown[1].permille, None, "an achievement the percentages do not name has no share");
        let facts: Vec<_> = hub.facts.iter().map(|f| (f.label.as_str(), f.value.as_str())).collect();
        assert_eq!(facts, [("Ultrawide", "Yes"), ("60 FPS", "Yes"), ("Field of view setting", "With a fix or mod"), ("Full controller support", "Yes"), ("Controller remapping", "No")]);
        assert_eq!(hub.credits, [STEAM_CREDIT, PCGW_CREDIT]);
    }

    #[test]
    fn a_source_that_fails_leaves_only_its_own_part_empty() {
        let (client, _) = served(vec![("appdetails", ok(STORE))]);
        let hub = fetch(&client, &pace(), &Lookup { steam: Some(620), title: "Portal 2" }, 1);
        assert_eq!((hub.deck, hub.review.is_some(), hub.news.len(), hub.facts.len()), (None, false, 0, 0));
        assert!(!hub.description.is_empty());
        let (offline, _) = client_offline();
        assert!(fetch(&offline, &pace(), &Lookup { steam: Some(620), title: "Portal 2" }, 1).is_empty(), "nothing fetched: nothing to show");
    }
    fn client_offline() -> (Client<Fixed>, Arc<Mutex<Vec<String>>>) {
        served(vec![])
    }

    #[test]
    fn the_review_summary_follows_the_redirect_and_falls_back_to_the_store_page() {
        let moved = Response { status: 302, body: vec![], location: Some("https://store.steampowered.com/appreviews/620?json=1&x=1".into()) };
        let (client, seen) = served(vec![("x=1", ok(REVIEWS)), ("appreviews/620?json=1&num", moved)]);
        let review = reviews(&client, &pace(), 620).unwrap();
        assert_eq!((review.label.as_str(), review.percent), ("Overwhelmingly Positive", Some(97)));
        assert_eq!(seen.lock().unwrap().len(), 2);

        let front_page = Response { status: 302, body: vec![], location: Some("https://store.steampowered.com/".into()) };
        let page = r#"<span class="game_review_summary positive" itemprop="description">Very Positive</span>
            <span data-tooltip-html="98% of the 175,136 user reviews in your language are positive">"#;
        let (client, _) = served(vec![("appreviews", front_page), ("store.steampowered.com/app/620", ok(page))]);
        let review = reviews(&client, &pace(), 620).unwrap();
        assert_eq!((review.label.as_str(), review.percent, review.count), ("Very Positive", Some(98), 175_136));
    }

    #[test]
    fn a_wiki_page_counts_only_for_the_same_steam_id_or_the_exact_title() {
        let wrong = WIKI.replace("620", "999");
        let (client, _) = served(vec![("pcgamingwiki", ok(&wrong))]);
        assert!(fetch(&client, &pace(), &Lookup { steam: Some(620), title: "Portal 2" }, 1).facts.is_empty(), "another game's page is not used");
        let titled = WIKI.replace("steam appid  = 620", "release dates =");
        let (client, _) = served(vec![("pcgamingwiki", ok(&titled))]);
        assert_eq!(fetch(&client, &pace(), &Lookup { steam: None, title: "portal 2" }, 1).facts.len(), 5);
        assert!(fetch(&client, &pace(), &Lookup { steam: None, title: "Portal 3" }, 1).facts.is_empty());
    }

    #[test]
    fn a_fresh_copy_is_served_without_the_network_and_an_old_one_when_offline() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Mutex::new(Cache::open(dir.path().join("hub"), crate::CacheLimits { bytes: 1 << 20, files: 8, image_bytes: 1 << 20 }).ok());
        let lookup = Lookup { steam: Some(620), title: "Portal 2" };
        let (online, _) = served(all());
        let first = load(&online, &pace(), &cache, &lookup, 1000).unwrap();
        let (offline, seen) = client_offline();
        assert_eq!(load(&offline, &pace(), &cache, &lookup, 2000).as_ref(), Some(&first));
        assert!(seen.lock().unwrap().is_empty(), "fresh: no request");
        let later = 1000 + FRESH_SECONDS + 1;
        assert_eq!(load(&offline, &pace(), &cache, &lookup, later).as_ref(), Some(&first), "stale and offline: the old copy");
        assert!(!seen.lock().unwrap().is_empty());
    }

    #[test]
    fn markup_dates_and_the_rate_gap() {
        assert_eq!(plain("a&nbsp;&amp;&nbsp;b<br/>[url=https://x]link[/url] {STEAM_CLAN_IMAGE}/p.png end"), "a & b link end");
        assert_eq!(plain("5 [of 7] things"), "5 [of 7] things", "only bracket codes go");
        assert_eq!(date(1_700_000_000), "14 Nov 2023");
        assert_eq!(clip("one two three four", 10), "one two…");
        let gap = Pacing::new(Duration::from_millis(50));
        let start = Instant::now();
        gap.wait("a");
        gap.wait("b");
        assert!(start.elapsed() < Duration::from_millis(40), "different hosts do not wait for each other");
        gap.wait("a");
        assert!(start.elapsed() >= Duration::from_millis(50), "one host does");
    }
}
