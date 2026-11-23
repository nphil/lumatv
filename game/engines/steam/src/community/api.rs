//! The JSON answers: reviews, news, how many play now, how rare each achievement is, and the store's pictures and trailers.
//!
//! Each is read loosely and one item at a time, so a field Steam changes costs that item and never the rest. An answer that cannot be
//! read at all is `Unreadable`; one that reads and holds nothing is `Empty`.

use super::text::{line, plain};
use super::{clamp32, https, listed, some};
use crate::http::Resp;
use lumagame_core::community::{
    Absence, MAX_API_NAME, MAX_NAME, MAX_NEWS, MAX_NEWS_TEXT, MAX_RARITY, MAX_REVIEW_TEXT, MAX_REVIEWS, MAX_SHOTS, MAX_TITLE, MAX_TRAILERS, NewsItem, Review,
    Reviews, Section, Shot, Trailer,
};
use serde_json::Value;
use std::collections::BTreeMap;

fn json(resp: &Resp) -> Option<Value> {
    serde_json::from_slice(&resp.body).ok()
}

/// A count Steam sends as a number, or as text in a few places; 0 when it is neither.
fn number(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn string(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or_default()
}

fn address(value: Option<&Value>) -> Option<String> {
    https(string(value))
}

/// `GetAppReviews`: the score and the most helpful reviews. Steam answers 200 with an empty `response` for a game it does not know.
pub(super) fn reviews(resp: &Resp) -> Section<Reviews> {
    let Some(body) = json(resp) else { return Section::absent(Absence::Unreadable) };
    let Some(summary) = body.pointer("/response/query_summary").filter(|summary| summary.is_object()) else {
        // A reply that says it succeeded and has no summary is a changed one; Steam's own refusal carries another result code.
        return Section::absent(if resp.eresult == Some(1) { Absence::Unreadable } else { Absence::Empty });
    };
    let (positive, negative) = (number(summary.get("total_positive")), number(summary.get("total_negative")));
    let total = positive.saturating_add(negative);
    let top: Vec<Review> = body.pointer("/response/reviews").and_then(Value::as_array).into_iter().flatten().take(MAX_REVIEWS).filter_map(review).collect();
    if total == 0 && top.is_empty() {
        return Section::absent(Absence::Empty);
    }
    let label = line(string(summary.get("review_score_desc")), MAX_NAME);
    Section::Present {
        value: Reviews {
            // "No user reviews" is Steam's word for no score; a label is only worth showing when there is one.
            label: Some(label).filter(|label| !label.is_empty() && !label.starts_with("No ")),
            percent: (total > 0).then(|| u8::try_from(positive.saturating_mul(100) / total).unwrap_or(100)),
            positive: clamp32(positive),
            negative: clamp32(negative),
            top,
        },
    }
}

fn review(value: &Value) -> Option<Review> {
    let text = plain(string(value.get("review")), MAX_REVIEW_TEXT);
    if text.is_empty() {
        return None;
    }
    Some(Review {
        text,
        recommended: value.get("voted_up")?.as_bool()?,
        helpful: clamp32(number(value.get("votes_up"))),
        funny: clamp32(number(value.get("votes_funny"))),
        // Steam counts minutes.
        playtime_s: number(value.pointer("/author/playtime_forever")).saturating_mul(60),
        at: number(value.get("timestamp_created")),
    })
}

/// `GetNewsForApp`: the newest posts first.
pub(super) fn news(resp: &Resp) -> Section<Vec<NewsItem>> {
    let Some(items) = json(resp).and_then(|mut body| body.pointer_mut("/appnews/newsitems").map(Value::take)).and_then(|items| match items {
        Value::Array(items) => Some(items),
        _ => None,
    }) else {
        return Section::absent(Absence::Unreadable);
    };
    let seen = items.len().min(MAX_NEWS);
    listed(seen, items.iter().take(MAX_NEWS).filter_map(news_item).collect())
}

fn news_item(value: &Value) -> Option<NewsItem> {
    let title = line(string(value.get("title")), MAX_TITLE);
    if title.is_empty() {
        return None;
    }
    Some(NewsItem {
        title,
        text: plain(string(value.get("contents")), MAX_NEWS_TEXT),
        author: some(line(string(value.get("author")), MAX_NAME)),
        source: some(line(string(value.get("feedlabel")), MAX_NAME)),
        at: number(value.get("date")),
    })
}

/// `GetNumberOfCurrentPlayers`. Steam answers 0 for a game it does not count, which is no answer.
pub(super) fn players(resp: &Resp) -> Section<u32> {
    let Some(body) = json(resp) else { return Section::absent(Absence::Unreadable) };
    let Some(response) = body.get("response").filter(|response| response.is_object()) else { return Section::absent(Absence::Unreadable) };
    match clamp32(number(response.get("player_count"))) {
        0 => Section::absent(Absence::Empty),
        value => Section::Present { value },
    }
}

/// `GetGlobalAchievementPercentagesForApp`: each achievement's share of players, in tenths of a percent.
pub(super) fn rarity(resp: &Resp) -> Section<BTreeMap<String, u16>> {
    let Some(Value::Array(items)) = json(resp).and_then(|mut body| body.pointer_mut("/achievementpercentages/achievements").map(Value::take)) else {
        return Section::absent(Absence::Unreadable);
    };
    let rarity: BTreeMap<String, u16> = items.iter().take(MAX_RARITY).filter_map(share).collect();
    if rarity.is_empty() {
        return Section::absent(if items.is_empty() { Absence::Empty } else { Absence::Unreadable });
    }
    Section::Present { value: rarity }
}

fn share(value: &Value) -> Option<(String, u16)> {
    let name = value.get("name")?.as_str()?;
    if name.is_empty() || name.len() > MAX_API_NAME || name.chars().any(char::is_control) {
        return None;
    }
    // The percentage is text in `GetGlobalAchievementPercentagesForApp` v2 (`"88.5"`) and a number in v1.
    let percent = match value.get("percent")? {
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        Value::Number(number) => number.as_f64()?,
        _ => return None,
    };
    percent.is_finite().then(|| (name.to_owned(), tenths(percent)))
}

fn tenths(percent: f64) -> u16 {
    (percent.clamp(0.0, 100.0) * 10.0).round() as u16
}

/// The store's `appdetails`: the page's screenshots and trailers, from one answer.
pub(super) fn store_media(resp: &Resp) -> (Section<Vec<Shot>>, Section<Vec<Trailer>>) {
    let both = |absence: Absence| (Section::absent(absence), Section::absent(absence));
    let Some(Value::Object(top)) = json(resp) else { return both(Absence::Unreadable) };
    // One game was asked for, so one entry answers.
    let Some(entry) = top.values().next() else { return both(Absence::Unreadable) };
    match entry.get("success").and_then(Value::as_bool) {
        Some(true) => {}
        // The store has no page for the game in this country.
        Some(false) => return both(Absence::Empty),
        None => return both(Absence::Unreadable),
    }
    let Some(data) = entry.get("data").filter(|data| data.is_object()) else { return both(Absence::Unreadable) };
    (shots(data.get("screenshots")), trailers(data.get("movies")))
}

/// The items of a store list: none when the page has no such key, which is how it says it has none.
fn store_list(value: Option<&Value>) -> Result<&[Value], Absence> {
    match value {
        None | Some(Value::Null) => Err(Absence::Empty),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(Absence::Unreadable),
    }
}

fn shots(value: Option<&Value>) -> Section<Vec<Shot>> {
    let items = match store_list(value) {
        Ok(items) => items,
        Err(absence) => return Section::absent(absence),
    };
    let read = |item: &Value| Some(Shot { thumb: address(item.get("path_thumbnail"))?, full: address(item.get("path_full"))? });
    listed(items.len().min(MAX_SHOTS), items.iter().take(MAX_SHOTS).filter_map(read).collect())
}

fn trailers(value: Option<&Value>) -> Section<Vec<Trailer>> {
    let items = match store_list(value) {
        Ok(items) => items,
        Err(absence) => return Section::absent(absence),
    };
    listed(items.len().min(MAX_TRAILERS), items.iter().take(MAX_TRAILERS).filter_map(trailer).collect())
}

/// H.264 only: every TV plays it. The AV1 and VP9 streams of the same trailer are left out.
fn trailer(item: &Value) -> Option<Trailer> {
    let (hls, dash) = (address(item.get("hls_h264")), address(item.get("dash_h264")));
    if hls.is_none() && dash.is_none() {
        return None;
    }
    Some(Trailer { title: line(string(item.get("name")), MAX_TITLE), thumb: address(item.get("thumbnail"))?, hls, dash })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake;

    fn ok(body: &str) -> Resp {
        fake::ok(body).unwrap()
    }

    fn eresult(code: i32, body: &str) -> Resp {
        fake::eresult(code, body).unwrap()
    }

    fn absent<T: std::fmt::Debug>(section: Section<T>) -> Absence {
        match section {
            Section::Absent { reason } => reason,
            present @ Section::Present { .. } => panic!("expected an absent section, got {present:?}"),
        }
    }

    fn present<T: std::fmt::Debug>(section: Section<T>) -> T {
        match section {
            Section::Present { value } => value,
            absent @ Section::Absent { .. } => panic!("expected a present section, got {absent:?}"),
        }
    }

    #[test]
    fn reviews_give_the_score_and_the_helpful_ones_without_names_or_markup() {
        let reviews = present(reviews(&ok(include_str!("fixtures/reviews.json"))));
        assert_eq!((reviews.label.as_deref(), reviews.percent, reviews.positive, reviews.negative), (Some("Very Positive"), Some(89), 361_150, 42_549));
        let texts: Vec<&str> = reviews.top.iter().map(|review| review.text.as_str()).collect();
        // The fourth has no text and the fifth no verdict: both are left out, the rest stay in Steam's order.
        assert_eq!(texts, ["Great story.\n\nThe city is alive. I put in 82 hours and would do it again.\n• Music\n• Writing", "Needs a patch. …", "Fine."]);
        let first = &reviews.top[0];
        assert_eq!((first.recommended, first.helpful, first.funny, first.playtime_s, first.at), (true, 171, 83, 4_961 * 60, 1_792_941_269));
        assert!(!reviews.top[1].recommended);
        let wire = serde_json::to_string(&reviews).unwrap();
        assert!(!wire.contains("steamid") && !wire.contains("recommendationid") && !wire.contains("hardware"), "{wire}");
    }

    #[test]
    fn a_game_without_reviews_has_no_label_and_no_percent() {
        let none = r#"{"success":1,"query_summary":{"num_reviews":0,"review_score":0,"review_score_desc":"No user reviews","total_positive":0,"total_negative":0,"total_reviews":0},"reviews":[]}"#;
        assert_eq!(absent(reviews(&ok(&format!(r#"{{"response":{none}}}"#)))), Absence::Empty);
        let few = r#"{"response":{"query_summary":{"review_score_desc":"No user reviews","total_positive":1,"total_negative":2},"reviews":[]}}"#;
        let reviews = present(reviews(&ok(few)));
        assert_eq!((reviews.label, reviews.percent, reviews.top.len()), (None, Some(33), 0));
    }

    #[test]
    fn reviews_that_are_not_what_they_were_are_unreadable_and_unknown_games_are_empty() {
        assert_eq!(absent(reviews(&ok("<html>"))), Absence::Unreadable);
        assert_eq!(absent(reviews(&ok(r#"{"reply":{}}"#))), Absence::Unreadable, "a success that has no summary changed");
        assert_eq!(absent(reviews(&eresult(42, r#"{"response":{}}"#))), Absence::Empty, "Steam's own 'no such game'");
        assert_eq!(absent(reviews(&ok(r#"{"response":{"query_summary":[]}}"#))), Absence::Unreadable);
    }

    #[test]
    fn one_odd_review_costs_only_itself() {
        let body = r#"{"response":{"query_summary":{"total_positive":"3","total_negative":1},"reviews":[
            {"review":"ok","voted_up":true,"votes_up":"7","author":{"playtime_forever":"60"}},
            "not an object", {"review":5,"voted_up":true}, {"review":"no verdict"}, {"review":"huge","voted_up":false,"votes_up":99999999999,"timestamp_created":"x"}]}}"#;
        let reviews = present(reviews(&ok(body)));
        assert_eq!((reviews.positive, reviews.negative, reviews.percent), (3, 1, Some(75)), "a count sent as text still counts");
        let kept: Vec<_> = reviews.top.iter().map(|review| (review.text.as_str(), review.helpful, review.playtime_s, review.at)).collect();
        assert_eq!(kept, [("ok", 7, 3_600, 0), ("huge", u32::MAX, 0, 0)]);
    }

    #[test]
    fn reviews_hold_no_more_than_the_limit_and_each_review_no_more_than_its_length() {
        let one = format!(r#"{{"review":"{}","voted_up":true}}"#, "word ".repeat(1_000));
        let body =
            format!(r#"{{"response":{{"query_summary":{{"total_positive":9,"total_negative":0}},"reviews":[{}]}}}}"#, vec![one; MAX_REVIEWS + 4].join(","));
        let reviews = present(reviews(&ok(&body)));
        assert_eq!(reviews.top.len(), MAX_REVIEWS);
        assert!(reviews.top.iter().all(|review| review.text.chars().count() <= MAX_REVIEW_TEXT && review.text.ends_with('…')));
        assert_eq!(reviews.percent, Some(100));
    }

    #[test]
    fn news_gives_title_text_author_source_and_time() {
        let items = present(news(&ok(include_str!("fixtures/news.json"))));
        assert_eq!(items.len(), 3, "the post with no title is left out");
        let first = &items[0];
        assert_eq!(first.title, "Edgerunners 2 Update (2.5) is out now!");
        assert_eq!((first.author.as_deref(), first.source.as_deref(), first.at), (Some("Studio Writer"), Some("Community Announcements"), 1_792_663_614));
        assert_eq!(first.text, "In the City of (Broken) Dreams, it’s not about how you flatline.\n\nRead the patch notes: what's new\n• New outfits\n• Fixes");
        // A third-party feed carries HTML.
        assert_eq!(items[1].text, "Seen from above & below. Read more");
        assert_eq!(items[1].author, None);
    }

    #[test]
    fn no_news_is_empty_and_a_changed_answer_is_unreadable() {
        assert_eq!(absent(news(&ok(r#"{"appnews":{"appid":1,"newsitems":[],"count":0}}"#))), Absence::Empty);
        assert_eq!(absent(news(&ok(r#"{"appnews":{"newsitems":[{"title":5},{"contents":"x"}]}}"#))), Absence::Unreadable, "posts that none can be read from");
        assert_eq!(absent(news(&ok(r#"{"news":[]}"#))), Absence::Unreadable);
        assert_eq!(absent(news(&ok(r#"{"appnews":{"newsitems":{}}}"#))), Absence::Unreadable);
        assert_eq!(absent(news(&ok(""))), Absence::Unreadable);
    }

    #[test]
    fn the_number_of_players_is_the_count_or_nothing() {
        assert_eq!(present(players(&ok(include_str!("fixtures/players.json")))), 47_861);
        assert_eq!(absent(players(&ok(r#"{"response":{"player_count":0,"result":1}}"#))), Absence::Empty);
        assert_eq!(absent(players(&ok(r#"{"response":{"result":42}}"#))), Absence::Empty);
        assert_eq!(present(players(&ok(r#"{"response":{"player_count":99999999999}}"#))), u32::MAX);
        assert_eq!(absent(players(&ok(r#"{"result":{}}"#))), Absence::Unreadable);
        assert_eq!(absent(players(&ok("nope"))), Absence::Unreadable);
    }

    #[test]
    fn rarity_is_in_tenths_of_a_percent_by_achievement_id() {
        let rarity = present(rarity(&ok(include_str!("fixtures/rarity.json"))));
        let rare = |name: &str| rarity.get(name).copied();
        assert_eq!(rare("TheFool"), Some(885), "text, as Steam sends it");
        assert_eq!(rare("Lover"), Some(673), "a number");
        assert_eq!(rare("Zero"), Some(0));
        assert_eq!(rare("Over"), Some(1_000), "clamped to 100 %");
        assert_eq!(rare("Tiny"), Some(1), "0.05 rounds up to a tenth");
        assert_eq!(rarity.len(), 5, "an entry with no name, a name with a control character and a percentage that is not a number are left out");
    }

    #[test]
    fn rarity_without_achievements_is_empty_and_a_changed_answer_is_unreadable() {
        assert_eq!(absent(rarity(&ok(r#"{"achievementpercentages":{"achievements":[]}}"#))), Absence::Empty);
        assert_eq!(absent(rarity(&ok(r#"{"achievementpercentages":{"achievements":[{"id":"x","pct":1}]}}"#))), Absence::Unreadable);
        assert_eq!(absent(rarity(&ok(r#"{"achievementpercentages":[]}"#))), Absence::Unreadable);
        assert_eq!(absent(rarity(&ok("{}"))), Absence::Unreadable);
    }

    #[test]
    fn rarity_keeps_the_most_it_may_and_names_of_a_sane_length_only() {
        let many: Vec<String> = (0..MAX_RARITY + 50).map(|n| format!(r#"{{"name":"A{n}","percent":"1"}}"#)).collect();
        let body =
            format!(r#"{{"achievementpercentages":{{"achievements":[{},{{"name":"{}","percent":"1"}}]}}}}"#, many.join(","), "n".repeat(MAX_API_NAME + 1));
        assert_eq!(present(rarity(&ok(&body))).len(), MAX_RARITY);
        let long = format!(r#"{{"achievementpercentages":{{"achievements":[{{"name":"{}","percent":"1"}}]}}}}"#, "n".repeat(MAX_API_NAME + 1));
        assert_eq!(absent(rarity(&ok(&long))), Absence::Unreadable);
    }

    #[test]
    fn the_store_gives_screenshots_and_h264_trailers() {
        let (shots, trailers) = store_media(&ok(include_str!("fixtures/appdetails.json")));
        let shots = present(shots);
        assert_eq!(shots.len(), 2, "a screenshot without a full-size address is left out");
        assert_eq!(shots[0].thumb, "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1091500/ss_1.600x338.jpg?t=1");
        assert_eq!(shots[0].full, "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1091500/ss_1.1920x1080.jpg?t=1");
        let trailers = present(trailers);
        assert_eq!(trailers.len(), 2, "a trailer with no H.264 stream is left out");
        assert_eq!(trailers[0].title, "Cyberpunk 2077 — Launch Trailer");
        assert_eq!(trailers[0].hls.as_deref(), Some("https://video.akamai.steamstatic.com/store_trailers/256972644/hls_264_master.m3u8?t=1"));
        assert_eq!(trailers[0].dash.as_deref(), Some("https://video.akamai.steamstatic.com/store_trailers/256972644/dash_h264.mpd?t=1"));
        assert_eq!(
            (trailers[1].hls.as_deref(), trailers[1].dash.as_deref()),
            (None, Some("https://video.akamai.steamstatic.com/store_trailers/2/dash_h264.mpd?t=1"))
        );
        let wire = serde_json::to_string(&trailers).unwrap();
        assert!(!wire.contains("av1") && !wire.contains("vp9"), "{wire}");
    }

    #[test]
    fn the_store_without_a_page_or_a_list_is_empty_and_a_changed_page_is_unreadable() {
        let (shots, trailers) = store_media(&ok(r#"{"999":{"success":false}}"#));
        assert_eq!((absent(shots), absent(trailers)), (Absence::Empty, Absence::Empty));
        let (shots, trailers) =
            store_media(&ok(r#"{"7":{"success":true,"data":{"screenshots":[{"path_thumbnail":"https://a/t.jpg","path_full":"https://a/f.jpg"}]}}}"#));
        assert_eq!((present(shots).len(), absent(trailers)), (1, Absence::Empty), "no `movies` key means no trailers");
        let (shots, trailers) = store_media(&ok(r#"{"7":{"success":true,"data":{"screenshots":{},"movies":[{"name":"x"}]}}}"#));
        assert_eq!((absent(shots), absent(trailers)), (Absence::Unreadable, Absence::Unreadable));
        for body in ["[]", "{}", r#"{"7":{"data":{}}}"#, r#"{"7":{"success":true}}"#, "<html>"] {
            let (shots, trailers) = store_media(&ok(body));
            assert_eq!((absent(shots), absent(trailers)), (Absence::Unreadable, Absence::Unreadable), "{body}");
        }
    }

    #[test]
    fn store_addresses_must_be_https_and_short() {
        let long = format!("https://a/{}", "x".repeat(600));
        let body = format!(
            r#"{{"7":{{"success":true,"data":{{"screenshots":[{{"path_thumbnail":"http://a/t.jpg","path_full":"https://a/f.jpg"}},{{"path_thumbnail":"https://a/t.jpg","path_full":"{long}"}},{{"path_thumbnail":"https://a/ t.jpg","path_full":"https://a/f.jpg"}}],"movies":[]}}}}}}"#
        );
        let (shots, _) = store_media(&ok(&body));
        assert_eq!(absent(shots), Absence::Unreadable);
    }
}
